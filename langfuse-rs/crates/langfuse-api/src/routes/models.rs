use axum::{Router, extract::Query, http::StatusCode, response::Json, routing::get, Extension};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa::IntoParams;
use langfuse_auth::jwt::Session;
use langfuse_db::repos::models::{self, TierWithPrices};
use crate::app::AppState;
use crate::middleware::auth::require_project_access;

#[derive(Debug, Deserialize, ToSchema, IntoParams)] pub struct ListQuery {
    pub project_id: String, pub limit: Option<i64>, pub page: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)] struct ModelResponse {
    pub id: String, pub project_id: Option<String>, pub model_name: String,
    pub match_pattern: String, pub start_date: Option<String>,
    pub input_price: Option<f64>, pub output_price: Option<f64>,
    pub total_price: Option<f64>, pub unit: Option<String>,
    pub tokenizer_id: Option<String>, pub tokenizer_config: Option<serde_json::Value>,
    pub created_at: String, pub updated_at: String,
}

/// Serialize a tier in the shape the model settings UI reads.
///
/// Two things are deliberate here. `prices` is keyed by usage type
/// (`cache_read_input_tokens`, …) — those keys are data, not field names, so
/// they are emitted exactly as stored; the client carries the object through
/// without renaming its keys. And each price is a bare number, not an object:
/// the models table feeds `prices` straight into `getMaxDecimals`/`new Decimal`,
/// so an `{ price: n }` wrapper arrives as `[object Object]` and throws.
///
/// The public `/api/public/models` response keeps the wrapped form, which is its
/// own documented contract.
fn tier_json(tier: &TierWithPrices) -> serde_json::Value {
    let prices: serde_json::Map<String, serde_json::Value> = tier
        .prices
        .iter()
        .map(|(usage_type, price)| (usage_type.clone(), serde_json::json!(decimal_f64(*price))))
        .collect();

    serde_json::json!({
        "id": tier.id,
        "name": tier.name,
        "isDefault": tier.is_default,
        "priority": tier.priority,
        "conditions": tier.conditions,
        "prices": prices,
    })
}

fn decimal_f64(value: rust_decimal::Decimal) -> f64 {
    langfuse_core::to_json_f64_or_zero(value)
}

/// List models for a project
#[utoipa::path(
    get,
    path = "/api/models",
    params(ListQuery),
    responses(
        (status = 200, description = "OK"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    ),
    security(
        ("bearer_auth" = []),
    ),
)]

pub(crate) async fn list(Extension(session): Extension<Session>, Query(q): Query<ListQuery>,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    require_project_access(&state.pool, &session, &q.project_id).await?;

    let limit = q.limit.unwrap_or(50).clamp(1, 100);
    let page = q.page.unwrap_or(1).max(1);

    // Project models plus the built-in definitions, matching upstream's model
    // settings list — a project's own row wins when both match.
    let (rows, total) = models::list_models(&state.pool, &q.project_id, page, limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let ids: Vec<String> = rows.iter().map(|m| m.id.clone()).collect();
    let mut tiers = models::tiers_for_models(&state.pool, &ids)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let data: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|m| {
            let response = ModelResponse {
                id: m.id.clone(),
                project_id: m.project_id.clone(),
                model_name: m.model_name,
                match_pattern: m.match_pattern,
                start_date: m.start_date.map(|d| d.and_utc().to_rfc3339()),
                input_price: m.input_price.map(decimal_f64),
                output_price: m.output_price.map(decimal_f64),
                total_price: m.total_price.map(decimal_f64),
                unit: m.unit,
                tokenizer_id: m.tokenizer_id,
                tokenizer_config: m.tokenizer_config,
                created_at: m.created_at.and_utc().to_rfc3339(),
                updated_at: m.created_at.and_utc().to_rfc3339(),
            };
            let model_tiers: Vec<serde_json::Value> = tiers
                .remove(&m.id)
                .unwrap_or_default()
                .iter()
                .map(tier_json)
                .collect();

            let mut value = serde_json::to_value(response).unwrap_or_default();
            if let Some(object) = value.as_object_mut() {
                object.insert("pricingTiers".to_string(), serde_json::json!(model_tiers));
            }
            value
        })
        .collect();

    Ok(Json(serde_json::json!({
        // `totalCount` sits inside `data` because the models table reads it from
        // the same payload it reads `models` from.
        "data": { "models": data, "totalCount": total },
        "meta": { "cursor": null, "has_more": (page * limit) < total, "total": total },
        "error": null,
    })))
}
pub fn router() -> Router<AppState> { Router::new().route("/", get(list)) }
