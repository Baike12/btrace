//! `GET /api/public/sessions` and `GET /api/public/sessions/{sessionId}`.
//!
//! A session groups every trace that shares a `session.id`, which is how LexQA's
//! per-conversation grouping surfaces: its HTTP middleware sets `session.id`
//! from the request, and the async worker spans inherit it.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension,
};
use chrono::NaiveDateTime;
use langfuse_auth::api_key::ApiKeyScope;
use langfuse_db::repos::{
    observations,
    sessions::{self, SessionRow},
    traces::{self, TraceListFilter},
};
use serde_json::json;

use crate::app::AppState;
use crate::public_api::traces::trace_json;
use crate::response::public_error;

pub async fn list(
    State(state): State<AppState>,
    Extension(scope): Extension<ApiKeyScope>,
    uri: axum::http::Uri,
) -> Response {
    let Some(project_id) = scope.project_id.clone() else {
        return public_error(StatusCode::BAD_REQUEST, "API key is not project-scoped", "bad_request");
    };

    let pairs = super::traces::parse_query_pairs(uri.query().unwrap_or(""));

    let mut page = 1i64;
    let mut limit = 50i64;
    let mut from: Option<NaiveDateTime> = None;
    let mut to: Option<NaiveDateTime> = None;

    for (key, value) in &pairs {
        let parsed = match key.as_str() {
            "page" => value
                .parse::<i64>()
                .map(|v| page = v)
                .map_err(|_| format!("Invalid page: {:?}", value)),
            "limit" => value
                .parse::<i64>()
                .map(|v| limit = v)
                .map_err(|_| format!("Invalid limit: {:?}", value)),
            "fromTimestamp" => super::traces::parse_timestamp(value).map(|v| from = Some(v)),
            "toTimestamp" => super::traces::parse_timestamp(value).map(|v| to = Some(v)),
            // No environment column on `trace_sessions` filtering in this fork.
            "environment" => Ok(()),
            other => Err(format!("Unknown query parameter: {}", other)),
        };
        if let Err(message) = parsed {
            return public_error(StatusCode::BAD_REQUEST, message, "bad_request");
        }
    }

    let limit = limit.clamp(1, 100);
    let rows = match sessions::list_sessions_filtered(&state.pool, &project_id, from, to, limit).await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "public sessions: list failed");
            return public_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to list sessions", "internal_error");
        }
    };

    let total = sessions::count_sessions(&state.pool, &project_id, from, to)
        .await
        .unwrap_or(0);
    let total_pages = if total == 0 { 0 } else { (total + limit - 1) / limit };

    let data: Vec<_> = rows.iter().map(session_json).collect();

    (
        StatusCode::OK,
        axum::Json(json!({
            "data": data,
            "meta": { "page": page.max(1), "limit": limit, "totalItems": total, "totalPages": total_pages },
        })),
    )
        .into_response()
}

pub async fn get(
    State(state): State<AppState>,
    Extension(scope): Extension<ApiKeyScope>,
    Path(session_id): Path<String>,
) -> Response {
    let Some(project_id) = scope.project_id.clone() else {
        return public_error(StatusCode::BAD_REQUEST, "API key is not project-scoped", "bad_request");
    };

    let session = match sessions::find_session(&state.pool, &project_id, &session_id).await {
        Ok(Some(session)) => session,
        Ok(None) => {
            return public_error(
                StatusCode::NOT_FOUND,
                format!("Session {} not found", session_id),
                "not_found",
            )
        }
        Err(e) => {
            tracing::error!(error = %e, "public sessions: lookup failed");
            return public_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to load session", "internal_error");
        }
    };

    // The session's traces are the traces carrying this session id, newest first.
    let filter = TraceListFilter {
        session_id: Some(session_id.clone()),
        page: 1,
        limit: 100,
        ..Default::default()
    };

    let traces = match traces::list_traces_filtered(&state.pool, &project_id, &filter).await {
        Ok((traces, _)) => traces,
        Err(e) => {
            tracing::error!(error = %e, "public sessions: traces lookup failed");
            Vec::new()
        }
    };

    let ids: Vec<String> = traces.iter().map(|t| t.id.clone()).collect();
    let rollups = observations::trace_rollups(&state.pool, &project_id, &ids)
        .await
        .unwrap_or_default();

    let mut body = session_json(&session);
    if let Some(obj) = body.as_object_mut() {
        obj.insert(
            "traces".to_string(),
            serde_json::Value::Array(
                traces
                    .iter()
                    .map(|t| trace_json(t, false, rollups.get(&t.id), &[], &[]))
                    .collect(),
            ),
        );
    }

    (StatusCode::OK, axum::Json(body)).into_response()
}

pub(crate) fn session_json(row: &SessionRow) -> serde_json::Value {
    json!({
        "id": row.id,
        "projectId": row.project_id,
        "environment": row.environment,
        "createdAt": row.created_at.and_utc().to_rfc3339(),
        "updatedAt": row.updated_at.and_utc().to_rfc3339(),
    })
}
