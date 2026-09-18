//! `GET /api/public/traces` and `GET /api/public/traces/{traceId}`.
//!
//! Implements the Langfuse public trace contract on top of the same
//! `langfuse_db::repos` queries the private UI routes use, so the two cannot
//! drift into different notions of what a filter means.
//!
//! Responses are camelCase per the public contract — the private routes emit
//! snake_case and rely on the frontend's key conversion, which machine clients
//! do not do.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension,
};
use chrono::NaiveDateTime;
use langfuse_auth::api_key::ApiKeyScope;
use langfuse_db::repos::{
    observations::{self, ObservationRow},
    scores::{self, ScoreRow},
    traces::{self, TraceListFilter, TraceRow},
};
use serde_json::{json, Map, Value};

use crate::app::AppState;
use crate::response::public_error;

/// Query parameters are read as raw pairs so that repeated keys — `tags=a&tags=b`,
/// which the contract declares with `allow-multiple` — are preserved. A typed
/// `Query<T>` would keep only the last occurrence.
#[derive(Debug, Default)]
pub struct ListTracesParams {
    page: Option<i64>,
    limit: Option<i64>,
    user_id: Option<String>,
    name: Option<String>,
    session_id: Option<String>,
    from_timestamp: Option<String>,
    to_timestamp: Option<String>,
    order_by: Option<String>,
    tags: Vec<String>,
    version: Option<String>,
    release: Option<String>,
}

impl ListTracesParams {
    fn parse(pairs: &[(String, String)]) -> Result<Self, String> {
        let mut params = Self::default();

        for (key, value) in pairs {
            match key.as_str() {
                "page" => params.page = Some(parse_int(value, "page")?),
                "limit" => params.limit = Some(parse_int(value, "limit")?),
                "userId" => params.user_id = Some(value.clone()),
                "name" => params.name = Some(value.clone()),
                "sessionId" => params.session_id = Some(value.clone()),
                "fromTimestamp" => params.from_timestamp = Some(value.clone()),
                "toTimestamp" => params.to_timestamp = Some(value.clone()),
                "orderBy" => params.order_by = Some(value.clone()),
                "version" => params.version = Some(value.clone()),
                "release" => params.release = Some(value.clone()),
                "tags" => {
                    // Tolerate both `tags=a,b` and repeated `tags=a&tags=b`.
                    params.tags.extend(
                        value
                            .split(',')
                            .map(str::trim)
                            .filter(|t| !t.is_empty())
                            .map(str::to_string),
                    );
                }
                // `environment` has no column on `traces` in this fork; accepted
                // and ignored rather than rejected, so a client that always
                // sends it still works.
                "environment" | "fields" => {}
                other => return Err(format!("Unknown query parameter: {}", other)),
            }
        }

        Ok(params)
    }

    fn into_filter(self) -> Result<TraceListFilter, String> {
        Ok(TraceListFilter {
            user_id: self.user_id,
            session_id: self.session_id,
            name: self.name,
            tags: self.tags,
            version: self.version,
            release: self.release,
            from_timestamp: self.from_timestamp.as_deref().map(parse_timestamp).transpose()?,
            to_timestamp: self.to_timestamp.as_deref().map(parse_timestamp).transpose()?,
            order_by: self.order_by.as_deref().map(parse_order_by).transpose()?,
            page: self.page.unwrap_or(1),
            limit: self.limit.unwrap_or(50),
        })
    }
}

/// `orderBy` is `[field].[asc|desc]` with camelCase fields.
fn parse_order_by(raw: &str) -> Result<(String, bool), String> {
    let (field, direction) = raw
        .split_once('.')
        .ok_or_else(|| format!("Invalid orderBy {:?}; expected [field].[asc|desc]", raw))?;

    let column = match field {
        "id" => "id",
        "timestamp" => "timestamp",
        "name" => "name",
        "userId" => "user_id",
        "release" => "release",
        "version" => "version",
        "public" => "public",
        "bookmarked" => "bookmarked",
        "sessionId" => "session_id",
        other => return Err(format!("Cannot order traces by {:?}", other)),
    };

    let ascending = match direction {
        "asc" => true,
        "desc" => false,
        other => return Err(format!("Invalid order direction {:?}", other)),
    };

    Ok((column.to_string(), ascending))
}

fn parse_int(value: &str, field: &str) -> Result<i64, String> {
    value
        .parse()
        .map_err(|_| format!("Invalid {}: {:?} is not an integer", field, value))
}

pub(crate) fn parse_timestamp(value: &str) -> Result<NaiveDateTime, String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.naive_utc())
        .map_err(|_| format!("Invalid timestamp {:?}; expected ISO 8601", value))
}

pub async fn list(
    State(state): State<AppState>,
    Extension(scope): Extension<ApiKeyScope>,
    uri: axum::http::Uri,
) -> Response {
    let Some(project_id) = scope.project_id.clone() else {
        return public_error(StatusCode::BAD_REQUEST, "API key is not project-scoped", "bad_request");
    };

    let pairs = parse_query_pairs(uri.query().unwrap_or(""));
    let filter = match ListTracesParams::parse(&pairs).and_then(ListTracesParams::into_filter) {
        Ok(filter) => filter,
        Err(message) => return public_error(StatusCode::BAD_REQUEST, message, "bad_request"),
    };

    let (rows, total) = match traces::list_traces_filtered(&state.pool, &project_id, &filter).await {
        Ok(result) => result,
        Err(e) => {
            tracing::error!(error = %e, "public traces: list failed");
            return public_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to list traces", "internal_error");
        }
    };

    let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    let rollups = observations::trace_rollups(&state.pool, &project_id, &ids)
        .await
        .unwrap_or_default();

    let data: Vec<Value> = rows
        .iter()
        .map(|row| trace_json(row, false, rollups.get(&row.id), &[], &[]))
        .collect();

    let limit = filter.limit.clamp(1, 100);
    let page = filter.page.max(1);
    let total_pages = if total == 0 { 0 } else { (total + limit - 1) / limit };

    (
        StatusCode::OK,
        axum::Json(json!({
            "data": data,
            "meta": { "page": page, "limit": limit, "totalItems": total, "totalPages": total_pages },
        })),
    )
        .into_response()
}

pub async fn get(
    State(state): State<AppState>,
    Extension(scope): Extension<ApiKeyScope>,
    Path(trace_id): Path<String>,
) -> Response {
    let Some(project_id) = scope.project_id.clone() else {
        return public_error(StatusCode::BAD_REQUEST, "API key is not project-scoped", "bad_request");
    };

    let trace = match traces::find_by_id(&state.pool, &trace_id, &project_id).await {
        Ok(Some(trace)) => trace,
        Ok(None) => {
            return public_error(
                StatusCode::NOT_FOUND,
                format!("Trace {} not found", trace_id),
                "not_found",
            )
        }
        Err(e) => {
            tracing::error!(error = %e, "public traces: lookup failed");
            return public_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to load trace", "internal_error");
        }
    };

    let (observations, scores) = tokio::join!(
        observations::list_observations(&state.pool, &project_id, Some(&trace_id), None, None, 1000),
        scores::find_by_trace_id(&state.pool, &trace_id, &project_id),
    );

    let observations = observations.unwrap_or_default();
    let scores = scores.unwrap_or_default();

    let rollups = observations::trace_rollups(
        &state.pool,
        &project_id,
        std::slice::from_ref(&trace_id),
    )
    .await
    .unwrap_or_default();

    let body = trace_json(
        &trace,
        true,
        rollups.get(&trace_id),
        &observations,
        &scores,
    );

    (StatusCode::OK, axum::Json(body)).into_response()
}

/// Split a raw query string into decoded key/value pairs, preserving repeats.
pub(crate) fn parse_query_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(key), percent_decode(value))
        })
        .collect()
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(&input[i + 1..i + 3], 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Render a trace in the public contract's camelCase shape.
pub(crate) fn trace_json(
    row: &TraceRow,
    include_children: bool,
    rollup: Option<&observations::TraceRollup>,
    observations: &[ObservationRow],
    scores: &[ScoreRow],
) -> Value {
    let mut body = Map::new();
    body.insert("id".into(), json!(row.id));
    body.insert("timestamp".into(), json!(row.timestamp.and_utc().to_rfc3339()));
    body.insert("name".into(), json!(row.name));
    body.insert("input".into(), json!(row.input));
    body.insert("output".into(), json!(row.output));
    body.insert("metadata".into(), json!(row.metadata));
    body.insert("userId".into(), json!(row.user_id));
    body.insert("sessionId".into(), json!(row.session_id));
    body.insert("release".into(), json!(row.release));
    body.insert("version".into(), json!(row.version));
    body.insert("public".into(), json!(row.public));
    body.insert("bookmarked".into(), json!(row.bookmarked));
    body.insert("tags".into(), json!(row.tags));
    body.insert("externalId".into(), json!(row.external_id));
    // There is no environment column on `traces` in this fork; sessions carry
    // it. Report the placeholder rather than omitting the contract field.
    body.insert("environment".into(), json!("default"));
    body.insert("createdAt".into(), json!(row.created_at.and_utc().to_rfc3339()));
    body.insert("updatedAt".into(), json!(row.updated_at.and_utc().to_rfc3339()));

    let latency = rollup.and_then(|r| r.latency_ms);
    let total_cost = rollup
        .and_then(|r| r.total_cost)
        .map(|c| c.to_string().parse::<f64>().unwrap_or(0.0));
    body.insert("latency".into(), json!(latency));
    body.insert("totalCost".into(), json!(total_cost));

    if include_children {
        body.insert(
            "observations".into(),
            Value::Array(observations.iter().map(observation_json).collect()),
        );
        body.insert(
            "scores".into(),
            Value::Array(scores.iter().map(score_json).collect()),
        );
    } else {
        // The contract reports empty arrays for children that were not fetched.
        body.insert("observations".into(), json!([]));
        body.insert("scores".into(), json!([]));
    }

    // Convenience links the contract also returns.
    body.insert("htmlPath".into(), json!(format!("/project/x/traces/{}", row.id)));

    Value::Object(body)
}

pub(crate) fn observation_json(row: &ObservationRow) -> Value {
    json!({
        "id": row.id,
        "traceId": row.trace_id,
        "type": row.obs_type,
        "name": row.name,
        "startTime": row.start_time.map(|t| t.and_utc().to_rfc3339()),
        "endTime": row.end_time.map(|t| t.and_utc().to_rfc3339()),
        "completionStartTime": row.completion_start_time.map(|t| t.and_utc().to_rfc3339()),
        "model": row.model,
        "internalModel": row.internal_model,
        "modelParameters": row.model_parameters,
        "input": row.input,
        "output": row.output,
        "metadata": row.metadata,
        "parentObservationId": row.parent_observation_id,
        "level": row.level,
        "statusMessage": row.status_message,
        "version": row.version,
        "promptId": row.prompt_id,
        "usage": {
            "input": row.prompt_tokens,
            "output": row.completion_tokens,
            "total": row.total_tokens,
            "unit": row.unit,
        },
        "usageDetails": row.usage_details,
        "costDetails": row.cost_details,
        "calculatedInputCost": row.calculated_input_cost.map(|c| c.to_string().parse::<f64>().unwrap_or(0.0)),
        "calculatedOutputCost": row.calculated_output_cost.map(|c| c.to_string().parse::<f64>().unwrap_or(0.0)),
        "calculatedTotalCost": row.calculated_total_cost.map(|c| c.to_string().parse::<f64>().unwrap_or(0.0)),
        "latency": latency_ms(row),
    })
}

fn latency_ms(row: &ObservationRow) -> Option<f64> {
    let start = row.start_time?;
    let end = row.end_time?;
    Some((end - start).num_milliseconds() as f64)
}

fn score_json(row: &ScoreRow) -> Value {
    json!({
        "id": row.id,
        "traceId": row.trace_id,
        "observationId": row.observation_id,
        "name": row.name,
        "value": row.value,
        "stringValue": row.string_value,
        "source": row.source,
        "dataType": row.data_type,
        "comment": row.comment,
        "configId": row.config_id,
        "timestamp": row.timestamp.and_utc().to_rfc3339(),
        "authorUserId": row.author_user_id,
    })
}
