//! `/api/public/models` — model definitions and their prices.
//!
//! Machine clients configure pricing through this endpoint so costs are computed
//! at ingest. That is what makes `calculated_*_cost` meaningful for a caller like
//! LexQA, which cannot reach the browser UI.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension,
};
use langfuse_auth::api_key::ApiKeyScope;
use langfuse_db::repos::models::{self, CreateModelInput, ModelRow, PriceInput, PricingTierInput};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::app::AppState;
use crate::response::public_error;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateModelRequest {
    pub model_name: String,
    pub match_pattern: String,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub unit: Option<String>,
    /// Deprecated flat prices; they create the default pricing tier.
    #[serde(default)]
    pub input_price: Option<Decimal>,
    #[serde(default)]
    pub output_price: Option<Decimal>,
    #[serde(default)]
    pub total_price: Option<Decimal>,
    #[serde(default)]
    pub pricing_tiers: Option<Vec<PricingTierRequest>>,
    #[serde(default)]
    pub tokenizer_id: Option<String>,
    #[serde(default)]
    pub tokenizer_config: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingTierRequest {
    pub name: String,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "empty_conditions")]
    pub conditions: Value,
    /// `usage_type` → price.
    #[serde(default)]
    pub prices: std::collections::BTreeMap<String, TierPriceRequest>,
}

#[derive(Debug, Deserialize)]
pub struct TierPriceRequest {
    #[serde(deserialize_with = "deserialize_decimal")]
    pub price: Decimal,
}

fn empty_conditions() -> Value {
    json!([])
}

/// Prices arrive as JSON numbers; `Decimal` is parsed from a string to avoid
/// binary-float drift on values like `0.000002`.
///
/// Both spellings a JSON number can take have to be accepted: `serde_json`
/// renders a small value as `2e-7`, and `Decimal::from_str` rejects exponent
/// notation — so a legitimately tiny price would otherwise be a 422.
fn deserialize_decimal<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    let raw = match value {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s,
        _ => return Err(serde::de::Error::custom("price must be a number")),
    };

    raw.parse::<Decimal>()
        .or_else(|_| Decimal::from_scientific(&raw))
        .map_err(|_| serde::de::Error::custom(format!("invalid price: {}", raw)))
}

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
    for (key, value) in &pairs {
        match key.as_str() {
            "page" => match value.parse() {
                Ok(v) => page = v,
                Err(_) => return public_error(StatusCode::BAD_REQUEST, "Invalid page", "bad_request"),
            },
            "limit" => match value.parse() {
                Ok(v) => limit = v,
                Err(_) => return public_error(StatusCode::BAD_REQUEST, "Invalid limit", "bad_request"),
            },
            other => {
                return public_error(
                    StatusCode::BAD_REQUEST,
                    format!("Unknown query parameter: {}", other),
                    "bad_request",
                )
            }
        }
    }

    let (rows, total) = match models::list_models(&state.pool, &project_id, page, limit).await {
        Ok(result) => result,
        Err(e) => {
            tracing::error!(error = %e, "public models: list failed");
            return public_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to list models", "internal_error");
        }
    };

    let limit = limit.clamp(1, 100);
    let total_pages = if total == 0 { 0 } else { (total + limit - 1) / limit };

    (
        StatusCode::OK,
        axum::Json(json!({
            "data": rows.iter().map(model_json).collect::<Vec<_>>(),
            "meta": { "page": page.max(1), "limit": limit, "totalItems": total, "totalPages": total_pages },
        })),
    )
        .into_response()
}

pub async fn get(
    State(state): State<AppState>,
    Extension(scope): Extension<ApiKeyScope>,
    Path(id): Path<String>,
) -> Response {
    let Some(project_id) = scope.project_id.clone() else {
        return public_error(StatusCode::BAD_REQUEST, "API key is not project-scoped", "bad_request");
    };

    match models::find_model(&state.pool, &project_id, &id).await {
        Ok(Some(model)) => (StatusCode::OK, axum::Json(model_json(&model))).into_response(),
        Ok(None) => public_error(StatusCode::NOT_FOUND, format!("Model {} not found", id), "not_found"),
        Err(e) => {
            tracing::error!(error = %e, "public models: lookup failed");
            public_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to load model", "internal_error")
        }
    }
}

pub async fn create(
    State(state): State<AppState>,
    Extension(scope): Extension<ApiKeyScope>,
    axum::Json(body): axum::Json<CreateModelRequest>,
) -> Response {
    let Some(project_id) = scope.project_id.clone() else {
        return public_error(StatusCode::BAD_REQUEST, "API key is not project-scoped", "bad_request");
    };

    // The contract forbids mixing the two pricing styles; silently preferring one
    // would discard prices the caller believes are stored.
    let has_flat = body.input_price.is_some() || body.output_price.is_some() || body.total_price.is_some();
    let has_tiers = body.pricing_tiers.as_ref().is_some_and(|t| !t.is_empty());
    if has_flat && has_tiers {
        return public_error(
            StatusCode::BAD_REQUEST,
            "Provide either flat prices (inputPrice/outputPrice/totalPrice) or pricingTiers, not both",
            "bad_request",
        );
    }

    let start_date = match body.start_date.as_deref() {
        Some(raw) => match super::traces::parse_timestamp(raw) {
            Ok(parsed) => Some(parsed),
            Err(message) => return public_error(StatusCode::BAD_REQUEST, message, "bad_request"),
        },
        None => None,
    };

    let mut flat_prices = Vec::new();
    for (usage_type, price) in [
        ("input", body.input_price),
        ("output", body.output_price),
        ("total", body.total_price),
    ] {
        if let Some(price) = price {
            flat_prices.push(PriceInput {
                usage_type: usage_type.to_string(),
                price,
            });
        }
    }

    let pricing_tiers = body
        .pricing_tiers
        .unwrap_or_default()
        .into_iter()
        .map(|tier| PricingTierInput {
            name: tier.name,
            is_default: tier.is_default,
            priority: tier.priority,
            conditions: tier.conditions,
            prices: tier
                .prices
                .into_iter()
                .map(|(usage_type, price)| PriceInput {
                    usage_type,
                    price: price.price,
                })
                .collect(),
        })
        .collect();

    let input = CreateModelInput {
        model_name: body.model_name,
        match_pattern: body.match_pattern,
        start_date,
        unit: body.unit,
        tokenizer_id: body.tokenizer_id,
        tokenizer_config: body.tokenizer_config,
        flat_prices,
        pricing_tiers,
    };

    match models::create_model(&state.pool, &project_id, &input).await {
        Ok(model) => (StatusCode::OK, axum::Json(model_json(&model))).into_response(),
        // A conflicting or malformed definition is the caller's to fix.
        Err(e @ langfuse_core::AppError::Conflict(_))
        | Err(e @ langfuse_core::AppError::Validation(_))
        | Err(e @ langfuse_core::AppError::BadRequest(_)) => {
            public_error(StatusCode::BAD_REQUEST, e.to_string(), "bad_request")
        }
        Err(e) => {
            tracing::error!(error = %e, "public models: create failed");
            public_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to create model",
                "internal_error",
            )
        }
    }
}

pub async fn delete(
    State(state): State<AppState>,
    Extension(scope): Extension<ApiKeyScope>,
    Path(id): Path<String>,
) -> Response {
    let Some(project_id) = scope.project_id.clone() else {
        return public_error(StatusCode::BAD_REQUEST, "API key is not project-scoped", "bad_request");
    };

    match models::delete_model(&state.pool, &project_id, &id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => match models::is_langfuse_managed(&state.pool, &id).await {
            Ok(true) => public_error(
                StatusCode::BAD_REQUEST,
                "Cannot delete a built-in model. Create a custom definition with the same model name to override it.",
                "bad_request",
            ),
            _ => public_error(StatusCode::NOT_FOUND, format!("Model {} not found", id), "not_found"),
        },
        Err(e) => {
            tracing::error!(error = %e, "public models: delete failed");
            public_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete model", "internal_error")
        }
    }
}

fn model_json(model: &ModelRow) -> Value {
    let mut prices = Map::new();
    for (usage_type, price) in &model.prices {
        prices.insert(usage_type.clone(), json!({ "price": decimal_to_f64(*price) }));
    }

    json!({
        "id": model.id,
        "modelName": model.model_name,
        "matchPattern": model.match_pattern,
        "startDate": model.start_date.map(|d| d.and_utc().to_rfc3339()),
        "unit": model.unit,
        // The deprecated flat fields are populated from the default tier, which
        // is what the contract says callers should see here.
        "inputPrice": model.input_price.map(decimal_to_f64),
        "outputPrice": model.output_price.map(decimal_to_f64),
        "totalPrice": model.total_price.map(decimal_to_f64),
        "tokenizerId": model.tokenizer_id,
        "tokenizerConfig": model.tokenizer_config,
        "isLangfuseManaged": model.is_langfuse_managed(),
        "createdAt": model.created_at.and_utc().to_rfc3339(),
        "prices": Value::Object(prices),
    })
}

fn decimal_to_f64(value: Decimal) -> f64 {
    value.to_string().parse().unwrap_or(0.0)
}
