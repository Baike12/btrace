//! Cost calculation from the project's configured model prices.
//!
//! Fills `calculated_*_cost` and `cost_details` on an observation. Those columns
//! existed before but nothing ever wrote them, so the Langfuse promise of
//! "configure a model's price and costs are computed automatically" did not
//! hold.
//!
//! ## The contract this reproduces
//!
//! This mirrors upstream `IngestionService.calculateUsageCosts` (the TypeScript
//! worker implementation that was removed with ClickHouse). The output shape is
//! what the read paths expect, so it is worth stating exactly:
//!
//! ```text
//! cost_details[k]     = price[usage_type = k] × usage_details[k]   for each k that has a price
//! cost_details.total  = cost_details.total ?? Σ cost_details
//! ```
//!
//! `cost_details` is keyed by **usage type**, not by a fixed input/output pair.
//! That is what lets the observation detail view itemise cache reads and cache
//! writes as their own cost lines instead of folding them into one input rate.
//! The headline numbers are then derived from it
//! (`observations_converters.ts` → `reduceUsageOrCostDetails`):
//!
//! ```text
//! inputCost  = Σ cost_details[k] where k.startsWith("input")
//! outputCost = Σ cost_details[k] where k.startsWith("output")
//! totalCost  = cost_details.total
//! ```
//!
//! The same reduction is applied here to fill `input_cost` / `output_cost` /
//! `total_cost`, and the read path in `langfuse-api` applies it again to
//! `cost_details`, so the stored columns and the derived values agree by
//! construction rather than by coincidence.
//!
//! Values are JSON **numbers**, not strings: the frontend's `CostDetails` schema
//! keeps an entry only `if (typeof value === "number")`, so a string-encoded
//! cost would be silently dropped from every cost breakdown in the UI.
//!
//! ## Tiers
//!
//! When a model carries pricing tiers, exactly one is selected, by
//! `matchPricingTier`'s rules: the non-default tiers in ascending `priority`,
//! first whose conditions all hold, else the default tier. Scanning every
//! `prices` row for the model instead would blend tiers that are meant to be
//! mutually exclusive.
//!
//! ## When the client already knows the cost
//!
//! If the span carried `langfuse.observation.cost_details`, that value is used
//! verbatim and nothing is calculated — upstream's rule is "if the user has
//! provided any cost point, do not calculate any other cost points".

use std::collections::{BTreeMap, HashMap};

use rust_decimal::Decimal;
use serde_json::{Map, Value};
use sqlx::{PgPool, Row};

use langfuse_core::ObservationRecord;

/// One `pricing_tiers` row with the prices that belong to it.
#[derive(Clone)]
struct PricedTier {
    id: String,
    is_default: bool,
    priority: i32,
    conditions: Vec<Condition>,
    /// `usage_type` → price per unit.
    prices: HashMap<String, Decimal>,
}

/// A `PricingTierCondition`: sum the usage keys matching `pattern` and compare
/// the sum to `value`.
#[derive(Clone)]
struct Condition {
    pattern: regex::Regex,
    operator: String,
    value: f64,
}

/// A model row that matched an observation.
#[derive(Clone)]
struct PricedModel {
    id: String,
    name: String,
    /// Fallback for models whose prices were never migrated into `prices`
    /// (the seeded `whisper-large-v3` is one): the unit is not a token, so
    /// there is nothing to match per usage type and the total is
    /// `total_price × usage_details.total`. See `price_usage`.
    total_price: Option<Decimal>,
    tiers: Vec<PricedTier>,
}

/// Compute and attach the cost fields for one observation.
///
/// A no-op when the observation carries neither usage nor a unit: there is
/// nothing to price, and skipping keeps the common case off the database.
pub async fn apply_pricing(pool: &PgPool, project_id: &str, observation: &mut ObservationRecord) {
    if observation.usage_details.is_none() && observation.unit.is_none() {
        return;
    }

    // A client-supplied cost wins outright. Normalising it still matters: the
    // total has to be present for the read path, and the API's own JSON numbers
    // survive `as_object` unchanged.
    if let Some(provided) = observation
        .cost_details
        .as_ref()
        .and_then(Value::as_object)
        .filter(|o| !o.is_empty())
    {
        let (input, output, total, details) = finalize(decimal_map(provided));
        observation.cost_details = Some(Value::Object(details));
        observation.input_cost = input;
        observation.output_cost = output;
        observation.total_cost = total;
        observation.calculated_input_cost = input;
        observation.calculated_output_cost = output;
        observation.calculated_total_cost = total;
        return;
    }

    let model_name = match observation.model.as_deref() {
        Some(name) if !name.is_empty() => name,
        _ => return,
    };

    let candidate = match find_priced_model(pool, project_id, model_name).await {
        Ok(Some(candidate)) => candidate,
        Ok(None) => return,
        Err(e) => {
            // Pricing is derived data: failing to compute it must not reject an
            // otherwise valid span.
            tracing::warn!(model = %model_name, error = %e, "pricing: lookup failed, leaving costs unset");
            return;
        }
    };

    let usage = observation
        .usage_details
        .as_ref()
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let priced = price_usage(&candidate, &usage);

    observation.internal_model = Some(candidate.name.clone());
    observation.internal_model_id = Some(candidate.id.clone());

    let Some(priced) = priced else { return };

    let (input, output, total, details) = finalize(priced);
    observation.input_cost = input;
    observation.output_cost = output;
    observation.total_cost = total;
    observation.calculated_input_cost = input;
    observation.calculated_output_cost = output;
    observation.calculated_total_cost = total;
    observation.cost_details = Some(Value::Object(details));
}

/// `usage_type` → cost, for every usage type the model has a price for.
///
/// Returns `None` when nothing could be priced, so the caller leaves the cost
/// columns NULL rather than writing a misleading zero.
///
/// The costs stay `Decimal` here rather than becoming JSON numbers straight
/// away: these values also fill the `calculated_*_cost` columns, and routing
/// them through `f64` first would store `0.002400000000000000200000000000`
/// instead of `0.0024`. Only the `cost_details` rendering is lossy, which is
/// what the browser does its arithmetic in anyway.
fn price_usage(model: &PricedModel, usage: &Map<String, Value>) -> Option<BTreeMap<String, Decimal>> {
    let mut details = BTreeMap::new();

    if let Some(tier) = match_tier(&model.tiers, usage) {
        for (usage_type, raw) in usage {
            let Some(price) = tier.prices.get(usage_type) else {
                continue;
            };
            let Some(quantity) = as_decimal(raw) else {
                continue;
            };
            details.insert(usage_type.clone(), *price * quantity);
        }
    }

    // Legacy flat prices. Rows created before pricing tiers exist (for example
    // the seeded `whisper-large-v3`, which carries `total_price` for its
    // `SECONDS` unit but has no `prices` row) would otherwise bill nothing.
    // This only runs when no tier price matched at all, so it cannot shadow a
    // configured tier.
    if details.is_empty() {
        let quantity = usage.get("total").and_then(as_decimal);
        if let (Some(price), Some(quantity)) = (model.total_price, quantity) {
            details.insert("total".to_string(), price * quantity);
        }
    }

    (!details.is_empty()).then_some(details)
}

/// Apply the read path's reduction to a priced breakdown: guarantee a `total`
/// entry, then derive the input/output columns from the key prefixes.
///
/// A prefix with no matching key yields `None`, not zero — that is what
/// `reduceUsageOrCostDetails` reports, and it is the difference between "this
/// observation has no output cost" and "this observation's output cost is
/// zero". A flat per-unit rate, for instance, has no input/output split at all.
fn finalize(
    mut priced: BTreeMap<String, Decimal>,
) -> (
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Map<String, Value>,
) {
    let total = match priced.get("total") {
        Some(total) => *total,
        None => {
            let total: Decimal = priced.values().sum();
            priced.insert("total".to_string(), total);
            total
        }
    };

    let sum_prefix = |prefix: &str| -> Option<Decimal> {
        let matched: Vec<Decimal> = priced
            .iter()
            .filter(|(usage_type, _)| usage_type.starts_with(prefix))
            .map(|(_, cost)| *cost)
            .collect();
        (!matched.is_empty()).then(|| matched.iter().sum())
    };

    let details: Map<String, Value> = priced
        .iter()
        .map(|(usage_type, cost)| (usage_type.clone(), json_number(*cost)))
        .collect();

    (
        sum_prefix("input"),
        sum_prefix("output"),
        Some(total),
        details,
    )
}

/// Decode a client-supplied `cost_details` object into exact decimals.
///
/// String values parse exactly; JSON numbers go through their decimal rendering,
/// which is the best available reading of a value the client chose to send as a
/// float.
fn decimal_map(object: &Map<String, Value>) -> BTreeMap<String, Decimal> {
    object
        .iter()
        .filter_map(|(k, v)| as_decimal(v).map(|d| (k.clone(), d)))
        .collect()
}

/// Select the tier to bill against — `matchPricingTier`'s algorithm.
fn match_tier<'a>(tiers: &'a [PricedTier], usage: &Map<String, Value>) -> Option<&'a PricedTier> {
    let mut ranked: Vec<&PricedTier> = tiers.iter().filter(|t| !t.is_default).collect();
    ranked.sort_by_key(|t| t.priority);

    for tier in ranked {
        // Upstream treats a condition list that is empty as *unmatched*, not as
        // vacuously true: `every` on an empty array would otherwise make a
        // condition-less non-default tier swallow every request.
        if tier.conditions.is_empty() {
            continue;
        }
        if tier
            .conditions
            .iter()
            .all(|condition| condition.matches(usage))
        {
            return Some(tier);
        }
    }

    tiers.iter().find(|t| t.is_default)
}

impl Condition {
    /// Sum every usage key matching the pattern and compare to the threshold.
    ///
    /// A malformed pattern fails the condition, matching upstream's fail-safe:
    /// a broken condition must not make an unrelated tier win.
    fn matches(&self, usage: &Map<String, Value>) -> bool {
        let sum: f64 = usage
            .iter()
            .filter(|(key, _)| self.pattern.is_match(key))
            .filter_map(|(_, value)| as_decimal(value))
            .map(|value| value.to_string().parse::<f64>().unwrap_or(0.0))
            .sum();

        match self.operator.as_str() {
            "gt" => sum > self.value,
            "gte" => sum >= self.value,
            "lt" => sum < self.value,
            "lte" => sum <= self.value,
            "eq" => sum == self.value,
            "neq" => sum != self.value,
            _ => false,
        }
    }
}

fn as_decimal(value: &Value) -> Option<Decimal> {
    match value {
        Value::Number(n) => n.to_string().parse().ok(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Render a `Decimal` as a JSON number.
///
/// `Decimal`'s own `Serialize` would emit a string, which the frontend's
/// `CostDetails` schema discards. The precision loss is the point: `f64` is the
/// representation the browser does its arithmetic in anyway.
///
/// `normalize` first, though. `to_f64` divides a mantissa of up to 96 bits by
/// `10^scale` in floating point, and `numeric(65,30)` hands back values scaled to
/// 30 decimals — so `1000` arrives as `1000.000000000000000000000000000000` and
/// converts to `1000.0000000000001`. Stripping the trailing zeros first makes the
/// conversion exact.
fn json_number(value: Decimal) -> Value {
    match langfuse_core::to_json_f64(value) {
        Some(f) => serde_json::Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        None => Value::Null,
    }
}

/// Find the best-matching priced model for a model name.
///
/// Project-specific rows win over global ones; within the same scope, the most
/// recent `start_date` wins, so a price change can be scheduled without deleting
/// history.
async fn find_priced_model(
    pool: &PgPool,
    project_id: &str,
    model_name: &str,
) -> Result<Option<PricedModel>, sqlx::Error> {
    let rows = sqlx::query(
        r#"SELECT id, model_name, match_pattern, total_price
           FROM models
           WHERE project_id = $1 OR project_id IS NULL
           ORDER BY (project_id IS NULL) ASC, start_date DESC NULLS LAST, created_at DESC"#,
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;

    for row in rows {
        let pattern: String = row.get("match_pattern");
        let Ok(regex) = regex::Regex::new(&pattern) else {
            tracing::warn!(pattern = %pattern, "pricing: model match_pattern is not a valid regex");
            continue;
        };
        if !regex.is_match(model_name) {
            continue;
        }

        let id: String = row.get("id");
        let tiers = load_tiers(pool, &id).await?;

        return Ok(Some(PricedModel {
            id,
            name: row.get("model_name"),
            total_price: row.get("total_price"),
            tiers,
        }));
    }

    Ok(None)
}

/// Load a model's tiers together with their prices.
async fn load_tiers(pool: &PgPool, model_id: &str) -> Result<Vec<PricedTier>, sqlx::Error> {
    let rows = sqlx::query(
        r#"SELECT t.id, t.is_default, t.priority, t.conditions, p.usage_type, p.price
           FROM pricing_tiers t
           LEFT JOIN prices p ON p.pricing_tier_id = t.id
           WHERE t.model_id = $1
           ORDER BY t.priority ASC"#,
    )
    .bind(model_id)
    .fetch_all(pool)
    .await?;

    let mut tiers: Vec<PricedTier> = Vec::new();
    for row in rows {
        let tier_id: String = row.get("id");
        // Rows arrive ordered by priority, and a tier's rows are contiguous.
        if tiers.last().map(|t| t.id.as_str()) != Some(tier_id.as_str()) {
            tiers.push(PricedTier {
                id: tier_id.clone(),
                is_default: row.get("is_default"),
                priority: row.get("priority"),
                conditions: parse_conditions(row.get("conditions")),
                prices: HashMap::new(),
            });
        }
        let Some(usage_type) = row.try_get::<Option<String>, _>("usage_type")? else {
            continue;
        };
        let price: Decimal = row.get("price");
        if let Some(tier) = tiers.last_mut() {
            tier.prices.insert(usage_type, price);
        }
    }

    Ok(tiers)
}

/// Decode the `conditions` jsonb array, dropping entries that cannot be
/// evaluated rather than failing the whole tier.
fn parse_conditions(value: Value) -> Vec<Condition> {
    let Some(entries) = value.as_array() else {
        return Vec::new();
    };

    entries
        .iter()
        .filter_map(|entry| {
            let pattern = entry.get("usageDetailPattern")?.as_str()?;
            let case_sensitive = entry
                .get("caseSensitive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let pattern = if case_sensitive {
                pattern.to_string()
            } else {
                format!("(?i){pattern}")
            };
            let pattern = regex::Regex::new(&pattern).ok()?;

            Some(Condition {
                pattern,
                operator: entry.get("operator")?.as_str()?.to_string(),
                value: entry.get("value")?.as_f64()?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tier(prices: &[(&str, i64)], is_default: bool, priority: i32) -> PricedTier {
        PricedTier {
            id: format!("tier-{priority}-{is_default}"),
            is_default,
            priority,
            conditions: Vec::new(),
            prices: prices
                .iter()
                .map(|(k, v)| (k.to_string(), Decimal::from(*v)))
                .collect(),
        }
    }

    fn usage(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    /// An already-priced breakdown, as `price_usage` would return it.
    fn priced(value: Value) -> BTreeMap<String, Decimal> {
        decimal_map(&usage(value))
    }

    #[test]
    fn default_tier_is_used_when_no_conditions_match() {
        let tiers = vec![tier(&[("input", 1)], true, 0)];
        let matched = match_tier(&tiers, &usage(json!({"input": 10}))).unwrap();
        assert!(matched.is_default);
    }

    #[test]
    fn a_conditional_tier_outranks_the_default() {
        let mut conditional = tier(&[("input", 2)], false, 1);
        conditional.conditions = vec![Condition {
            pattern: regex::Regex::new("(?i)input").unwrap(),
            operator: "gt".to_string(),
            value: 1_000.0,
        }];
        let tiers = vec![tier(&[("input", 1)], true, 0), conditional];

        assert_eq!(match_tier(&tiers, &usage(json!({"input": 2000})))
            .unwrap()
            .prices["input"], Decimal::from(2));
        assert_eq!(match_tier(&tiers, &usage(json!({"input": 10})))
            .unwrap()
            .prices["input"], Decimal::from(1));
    }

    #[test]
    fn lower_priority_wins_among_matching_conditional_tiers() {
        let mut first = tier(&[("input", 1)], false, 1);
        first.conditions = vec![Condition {
            pattern: regex::Regex::new("(?i)input").unwrap(),
            operator: "gte".to_string(),
            value: 1.0,
        }];
        let mut second = tier(&[("input", 9)], false, 2);
        second.conditions = first_conditions();
        let tiers = vec![first, second];

        assert_eq!(
            match_tier(&tiers, &usage(json!({"input": 5}))).unwrap().priority,
            1
        );
    }

    fn first_conditions() -> Vec<Condition> {
        vec![Condition {
            pattern: regex::Regex::new("(?i)input").unwrap(),
            operator: "gte".to_string(),
            value: 1.0,
        }]
    }

    #[test]
    fn a_tier_with_empty_conditions_never_matches() {
        let mut empty = tier(&[("input", 5)], false, 1);
        empty.conditions = Vec::new();
        let tiers = vec![empty, tier(&[("input", 1)], true, 0)];

        // Upstream treats empty conditions as unmatched, so the default applies.
        assert!(match_tier(&tiers, &usage(json!({"input": 10})))
            .unwrap()
            .is_default);
    }

    #[test]
    fn costs_are_keyed_by_usage_type_and_numbered() {
        let model = PricedModel {
            id: "m".into(),
            name: "qwen-max".into(),
            total_price: None,
            tiers: vec![tier(
                &[
                    ("input", 1),
                    ("output", 2),
                    ("cache_read_input_tokens", 1),
                ],
                true,
                0,
            )],
        };

        let details = price_usage(
            &model,
            &usage(json!({
                "input": 1200,
                "output": 300,
                "total": 1500,
                "cache_read_input_tokens": 800,
                "cache_miss_input_tokens": 300,
            })),
        )
        .unwrap();

        // `cache_miss_input_tokens` has no price, so it is absent rather than 0.
        assert_eq!(details.len(), 3);
        assert_eq!(details["cache_read_input_tokens"], Decimal::from(800));
        assert_eq!(details["input"], Decimal::from(1200));
    }

    #[test]
    fn costs_stay_exact_until_they_are_rendered() {
        // A rate of 0.000002 per token times 1200 is exactly 0.0024. Going via
        // f64 would store 0.0024000000000000002 in the numeric column.
        let mut priced_tier = tier(&[], true, 0);
        priced_tier
            .prices
            .insert("input".to_string(), Decimal::new(2, 6));

        let exact = price_usage(
            &PricedModel {
                id: "m".into(),
                name: "qwen-max".into(),
                total_price: None,
                tiers: vec![priced_tier],
            },
            &usage(json!({"input": 1200})),
        )
        .unwrap();
        assert_eq!(exact["input"], "0.0024".parse::<Decimal>().unwrap());

        let (input, _, _, details) = finalize(exact);
        assert_eq!(input.unwrap(), "0.0024".parse::<Decimal>().unwrap());
        assert_eq!(details["input"], json!(0.0024));
    }

    #[test]
    fn finalize_adds_total_and_splits_on_the_read_paths_prefixes() {
        let (input, output, total, details) = finalize(priced(json!({
            "input": 100,
            "output": 50,
            "cache_read_input_tokens": 25,
        })));

        // `cache_read_input_tokens` does not carry the `input` prefix, so it is
        // excluded from the column — exactly what `reduceUsageOrCostDetails`
        // does with `startsWith("input")`.
        assert_eq!(input.unwrap(), Decimal::from(100));
        assert_eq!(output.unwrap(), Decimal::from(50));
        assert_eq!(total.unwrap(), Decimal::from(175));
        assert_eq!(details["total"], json!(175.0));
    }

    #[test]
    fn a_breakdown_with_no_matching_prefix_reports_none_not_zero() {
        let (input, output, total, _) = finalize(priced(json!({"total": 0.00125})));
        assert_eq!(input, None, "a flat per-unit rate has no input/output split");
        assert_eq!(output, None);
        assert_eq!(total.unwrap(), "0.00125".parse::<Decimal>().unwrap());
    }

    #[test]
    fn high_scale_decimals_render_exactly() {
        // `numeric(65,30)` hands back 30 decimals' worth of trailing zeros. The
        // mantissa then exceeds f64's exact-integer range, so a naive `to_f64`
        // rounds: this rendered as 1000.0000000000001 before `normalize`.
        let from_pg = "1000.000000000000000000000000000000"
            .parse::<Decimal>()
            .unwrap();
        assert_eq!(json_number(from_pg), json!(1000.0));

        let cents = "0.002880000000000000000000000000"
            .parse::<Decimal>()
            .unwrap();
        assert_eq!(json_number(cents), json!(0.00288));
    }

    #[test]
    fn an_explicit_total_is_kept() {
        let (_, _, total, _) = finalize(priced(json!({"input": 10, "total": 99})));
        assert_eq!(total.unwrap(), Decimal::from(99));
    }

    #[test]
    fn legacy_flat_prices_still_bill_a_seconds_observation() {
        let model = PricedModel {
            id: "m".into(),
            name: "whisper-large-v3".into(),
            total_price: Some(Decimal::new(1, 4)),
            tiers: Vec::new(),
        };

        let details = price_usage(&model, &usage(json!({"total": 12}))).unwrap();
        assert_eq!(details["total"], "0.0012".parse::<Decimal>().unwrap());
    }

    #[test]
    fn numeric_strings_are_accepted_as_quantities() {
        let model = PricedModel {
            id: "m".into(),
            name: "m".into(),
            total_price: None,
            tiers: vec![tier(&[("input", 1)], true, 0)],
        };

        let details = price_usage(&model, &usage(json!({"input": "1200"}))).unwrap();
        assert_eq!(details["input"], Decimal::from(1200));
    }
}
