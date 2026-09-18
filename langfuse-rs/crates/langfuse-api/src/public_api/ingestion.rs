//! `POST /api/public/ingestion` — Langfuse SDK batch ingestion.
//!
//! Events are merged, mapped and written **in the request**, through the same
//! [`langfuse_ingestion::process_batch`] the queue consumer uses. Nothing is
//! enqueued: the previous implementation deferred every event by five seconds on
//! `pg_jobs`, so a trace was not queryable until well after the SDK's ingest
//! call returned and the delay was invisible to the caller.
//!
//! The response keeps the Langfuse contract: HTTP 207 with parallel
//! `successes` / `errors` arrays, so a partially valid batch reports which
//! events were rejected without discarding the rest.

use axum::{extract::State, http::StatusCode, response::Json, Extension};
use langfuse_auth::api_key::ApiKeyScope;
use langfuse_ingestion::service::process_batch;
use langfuse_ingestion::validator::{get_event_type, get_entity_type};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::AppState;

#[derive(Debug, Deserialize)]
pub struct IngestionRequest {
    pub batch: Vec<Value>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct IngestionResponse {
    pub successes: Vec<IngestionResult>,
    pub errors: Vec<IngestionError>,
}

#[derive(Debug, Serialize)]
pub struct IngestionResult {
    pub id: String,
    pub status: u16,
}

#[derive(Debug, Serialize)]
pub struct IngestionError {
    pub id: String,
    pub status: u16,
    pub message: String,
    pub error: Option<String>,
}

pub async fn handler(
    State(state): State<AppState>,
    Extension(scope): Extension<ApiKeyScope>,
    Json(body): Json<IngestionRequest>,
) -> Result<(StatusCode, Json<IngestionResponse>), (StatusCode, String)> {
    let project_id = scope
        .project_id
        .ok_or((StatusCode::BAD_REQUEST, "Missing project_id".to_string()))?;

    let mut successes = Vec::new();
    let mut errors = Vec::new();

    for event in &body.batch {
        let event_id = event.get("id").and_then(|i| i.as_str()).unwrap_or("unknown");

        let entity_type = match get_entity_type(event) {
            Some(entity_type) => entity_type,
            None => {
                errors.push(IngestionError {
                    id: event_id.to_string(),
                    status: 400,
                    message: format!(
                        "Could not determine entity type for event (type: {:?})",
                        get_event_type(event)
                    ),
                    error: Some("invalid_event_type".to_string()),
                });
                continue;
            }
        };

        // One entity per request event: the event list belongs to this event's
        // body, and `event_id` doubles as the dedup key so an SDK retry is a
        // no-op.
        match process_batch(
            &state.pool,
            &project_id,
            &entity_type,
            event_id,
            "",
            std::slice::from_ref(event),
        )
        .await
        {
            Ok(()) => successes.push(IngestionResult {
                id: event_id.to_string(),
                status: 201,
            }),
            Err(e) => errors.push(IngestionError {
                id: event_id.to_string(),
                status: 400,
                message: e.clone(),
                error: Some(e),
            }),
        }
    }

    // 207 is the Langfuse contract for this endpoint, including the all-success
    // case — SDKs read `successes` / `errors` from the body.
    Ok((
        StatusCode::MULTI_STATUS,
        Json(IngestionResponse { successes, errors }),
    ))
}

pub async fn health() -> &'static str {
    "OK"
}
