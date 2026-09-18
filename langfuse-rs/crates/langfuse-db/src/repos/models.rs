//! Model definitions and their prices, for `/api/public/models`.
//!
//! `models` holds the match rule (`match_pattern`, a regex over the observation's
//! model name); `prices` holds `usage_type` → unit price, grouped by
//! `pricing_tiers`. That split is what lets a cache read be priced differently
//! from a cache miss — the reason this endpoint exists for LexQA, which reports
//! `cache_read_input_tokens` and `cache_creation_input_tokens` separately.
//!
//! Every model gets a default pricing tier, because `prices` requires one
//! (`pricing_tier_id` is NOT NULL) and callers that only send flat prices should
//! not have to know that.

use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::{PgPool, Row};

use langfuse_core::Result;

/// A model definition with its default-tier prices.
#[derive(Debug, Clone)]
pub struct ModelRow {
    pub id: String,
    pub project_id: Option<String>,
    pub model_name: String,
    pub match_pattern: String,
    pub start_date: Option<chrono::NaiveDateTime>,
    pub unit: Option<String>,
    pub input_price: Option<Decimal>,
    pub output_price: Option<Decimal>,
    pub total_price: Option<Decimal>,
    pub tokenizer_id: Option<String>,
    pub tokenizer_config: Option<Value>,
    pub created_at: chrono::NaiveDateTime,
    /// `usage_type` → price, from the default pricing tier.
    pub prices: Vec<(String, Decimal)>,
}

impl ModelRow {
    /// Whether this is a built-in definition rather than a project's own.
    /// Built-ins are the rows with no `project_id`.
    pub fn is_langfuse_managed(&self) -> bool {
        self.project_id.is_none()
    }
}

const MODEL_SELECT: &str = r#"SELECT id, project_id, model_name, match_pattern, start_date, unit,
       input_price, output_price, total_price, tokenizer_id, tokenizer_config, created_at
FROM models"#;

/// A price to store for one usage type.
#[derive(Debug, Clone)]
pub struct PriceInput {
    pub usage_type: String,
    pub price: Decimal,
}

/// Everything `POST /api/public/models` accepts.
///
/// `pricing_tiers`, when provided, replaces the flat prices: the contract
/// forbids mixing the two. Each entry is `(tier name, is_default, prices)`.
#[derive(Debug, Clone, Default)]
pub struct CreateModelInput {
    pub model_name: String,
    pub match_pattern: String,
    pub start_date: Option<chrono::NaiveDateTime>,
    pub unit: Option<String>,
    pub tokenizer_id: Option<String>,
    pub tokenizer_config: Option<Value>,
    /// Flat `usage_type` → price pairs (the legacy `inputPrice`/`outputPrice`/
    /// `totalPrice` fields, plus any explicit per-usage-type prices).
    pub flat_prices: Vec<PriceInput>,
    pub pricing_tiers: Vec<PricingTierInput>,
}

#[derive(Debug, Clone)]
pub struct PricingTierInput {
    pub name: String,
    pub is_default: bool,
    pub priority: i32,
    pub conditions: Value,
    pub prices: Vec<PriceInput>,
}

/// List a project's models plus the built-in (project-less) ones, page/limit.
pub async fn list_models(
    pool: &PgPool,
    project_id: &str,
    page: i64,
    limit: i64,
) -> Result<(Vec<ModelRow>, i64)> {
    let limit = limit.clamp(1, 100);
    let offset = (page.max(1) - 1) * limit;

    let rows = sqlx::query(&format!(
        "{} WHERE project_id = $1 OR project_id IS NULL
         ORDER BY (project_id IS NULL) ASC, model_name ASC
         LIMIT $2 OFFSET $3",
        MODEL_SELECT
    ))
    .bind(project_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    let mut models = Vec::with_capacity(rows.len());
    for row in &rows {
        let id: String = row.get("id");
        let prices = default_tier_prices(pool, &id).await?;
        models.push(map_model_row(row, prices));
    }

    let total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM models WHERE project_id = $1 OR project_id IS NULL")
            .bind(project_id)
            .fetch_one(pool)
            .await?;

    Ok((models, total))
}

/// A pricing tier with its prices, in the shape the model settings UI reads.
#[derive(Debug, Clone)]
pub struct TierWithPrices {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub priority: i32,
    pub conditions: Value,
    /// `usage_type` → price per unit.
    pub prices: Vec<(String, Decimal)>,
}

/// Every pricing tier of several models at once, keyed by model id.
///
/// One query rather than one per model: the settings table lists up to a page of
/// models and would otherwise run a tier query per row.
pub async fn tiers_for_models(
    pool: &PgPool,
    model_ids: &[String],
) -> Result<std::collections::HashMap<String, Vec<TierWithPrices>>> {
    use std::collections::HashMap;

    let mut by_model: HashMap<String, Vec<TierWithPrices>> = HashMap::new();
    if model_ids.is_empty() {
        return Ok(by_model);
    }

    let rows = sqlx::query(
        r#"SELECT t.model_id, t.id, t.name, t.is_default, t.priority, t.conditions,
                  p.usage_type, p.price
           FROM pricing_tiers t
           LEFT JOIN prices p ON p.pricing_tier_id = t.id
           WHERE t.model_id = ANY($1)
           ORDER BY t.model_id, t.priority ASC"#,
    )
    .bind(model_ids)
    .fetch_all(pool)
    .await?;

    for row in rows {
        let model_id: String = row.get("model_id");
        let tier_id: String = row.get("id");

        let tiers = by_model.entry(model_id).or_default();
        if tiers.last().map(|t| t.id.as_str()) != Some(tier_id.as_str()) {
            tiers.push(TierWithPrices {
                id: tier_id.clone(),
                name: row.get("name"),
                is_default: row.get("is_default"),
                priority: row.get("priority"),
                conditions: row.get("conditions"),
                prices: Vec::new(),
            });
        }

        let Some(usage_type) = row.try_get::<Option<String>, _>("usage_type")? else {
            continue;
        };
        if let Some(tier) = tiers.last_mut() {
            tier.prices.push((usage_type, row.get("price")));
        }
    }

    Ok(by_model)
}

/// Fetch one model, scoped to the project (or a built-in).
pub async fn find_model(pool: &PgPool, project_id: &str, id: &str) -> Result<Option<ModelRow>> {
    let row = sqlx::query(&format!(
        "{} WHERE id = $1 AND (project_id = $2 OR project_id IS NULL)",
        MODEL_SELECT
    ))
    .bind(id)
    .bind(project_id)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(row) => {
            let prices = default_tier_prices(pool, id).await?;
            Ok(Some(map_model_row(&row, prices)))
        }
        None => Ok(None),
    }
}

/// Create a model and its prices.
///
/// Runs in one transaction: a model row whose prices failed to insert would
/// silently price nothing, which looks exactly like a model with no configured
/// rates.
pub async fn create_model(
    pool: &PgPool,
    project_id: &str,
    input: &CreateModelInput,
) -> Result<ModelRow> {
    // The `models` unique index covers (project_id, model_name, start_date,
    // unit), but PostgreSQL treats NULLs as distinct — so two models created
    // without a start date do not collide and the index cannot catch a
    // duplicate. Check explicitly, treating NULL as equal to NULL.
    let duplicate: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM models
               WHERE project_id = $1 AND model_name = $2
                 AND start_date IS NOT DISTINCT FROM $3
           )"#,
    )
    .bind(project_id)
    .bind(&input.model_name)
    .bind(input.start_date)
    .fetch_one(pool)
    .await?;

    if duplicate {
        return Err(langfuse_core::AppError::Conflict(format!(
            "A model named '{}' with the same start date already exists in this project",
            input.model_name
        )));
    }

    let mut tx = pool.begin().await?;
    let model_id = uuid::Uuid::new_v4().to_string();

    sqlx::query(
        r#"INSERT INTO models (
               id, project_id, model_name, match_pattern, start_date, unit,
               tokenizer_id, tokenizer_config, created_at, updated_at
           ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW(), NOW())"#,
    )
    .bind(&model_id)
    .bind(project_id)
    .bind(&input.model_name)
    .bind(&input.match_pattern)
    .bind(input.start_date)
    .bind(&input.unit)
    .bind(&input.tokenizer_id)
    .bind(&input.tokenizer_config)
    .execute(&mut *tx)
    .await?;

    // Flat prices land in a default tier named "Standard" — the same name the
    // contract documents for the auto-created tier.
    let tiers: Vec<PricingTierInput> = if input.pricing_tiers.is_empty() {
        vec![PricingTierInput {
            name: "Standard".to_string(),
            is_default: true,
            priority: 0,
            conditions: serde_json::json!([]),
            prices: input.flat_prices.clone(),
        }]
    } else {
        input.pricing_tiers.clone()
    };

    for tier in &tiers {
        let tier_id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            r#"INSERT INTO pricing_tiers (id, model_id, name, is_default, priority, conditions, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())"#,
        )
        .bind(&tier_id)
        .bind(&model_id)
        .bind(&tier.name)
        .bind(tier.is_default)
        .bind(tier.priority)
        .bind(&tier.conditions)
        .execute(&mut *tx)
        .await?;

        for price in &tier.prices {
            sqlx::query(
                r#"INSERT INTO prices (id, model_id, project_id, pricing_tier_id, usage_type, price, created_at, updated_at)
                   VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, NOW(), NOW())"#,
            )
            .bind(&model_id)
            .bind(project_id)
            .bind(&tier_id)
            .bind(&price.usage_type)
            .bind(price.price)
            .execute(&mut *tx)
            .await?;
        }

    }

    tx.commit().await?;

    find_model(pool, project_id, &model_id)
        .await?
        .ok_or_else(|| langfuse_core::AppError::Internal("created model not found".into()))
}

/// Delete a project's own model. Built-in definitions cannot be deleted —
/// matching the contract, which says to override them with a same-named custom
/// definition instead.
pub async fn delete_model(pool: &PgPool, project_id: &str, id: &str) -> Result<bool> {
    let result = sqlx::query("DELETE FROM models WHERE id = $1 AND project_id = $2")
        .bind(id)
        .bind(project_id)
        .execute(pool)
        .await?;

    Ok(result.rows_affected() > 0)
}

/// Whether the id names a built-in model, so the caller can explain a refusal.
pub async fn is_langfuse_managed(pool: &PgPool, id: &str) -> Result<bool> {
    let managed: Option<bool> = sqlx::query_scalar(
        "SELECT (project_id IS NULL) FROM models WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(managed.unwrap_or(false))
}

async fn default_tier_prices(pool: &PgPool, model_id: &str) -> Result<Vec<(String, Decimal)>> {
    let rows = sqlx::query(
        r#"SELECT p.usage_type, p.price
           FROM prices p
           JOIN pricing_tiers t ON t.id = p.pricing_tier_id
           WHERE p.model_id = $1 AND t.is_default = true
           ORDER BY p.usage_type"#,
    )
    .bind(model_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| (r.get::<String, _>("usage_type"), r.get::<Decimal, _>("price")))
        .collect())
}

fn map_model_row(row: &sqlx::postgres::PgRow, prices: Vec<(String, Decimal)>) -> ModelRow {
    ModelRow {
        id: row.get("id"),
        project_id: row.get("project_id"),
        model_name: row.get("model_name"),
        match_pattern: row.get("match_pattern"),
        start_date: row.get("start_date"),
        unit: row.get("unit"),
        input_price: row.get("input_price"),
        output_price: row.get("output_price"),
        total_price: row.get("total_price"),
        tokenizer_id: row.get("tokenizer_id"),
        tokenizer_config: row.get("tokenizer_config"),
        created_at: row.get("created_at"),
        prices,
    }
}
