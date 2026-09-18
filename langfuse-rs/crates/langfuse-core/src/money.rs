//! Decimal → `f64` conversion for JSON output.

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

/// Convert a money `Decimal` to the `f64` a JSON response carries.
///
/// `normalize` first, and that is the whole point of this function:
/// `Decimal::to_f64` divides a mantissa of up to 96 bits by `10^scale` in
/// floating point, and a `numeric(65,30)` column comes back scaled to 30
/// decimals. The mantissa of such a value exceeds `f64`'s 53-bit exact-integer
/// range, so the division rounds — `1000` retrieved from the database as
/// `1000.000000000000000000000000000000` converts to `1000.0000000000001`.
/// Stripping the trailing zeros brings the mantissa back into range and the
/// conversion becomes exact.
///
/// Returns `None` when the value cannot be represented (an overflow), so a
/// caller can decide between null and a saturating value.
pub fn to_json_f64(value: Decimal) -> Option<f64> {
    value.normalize().to_f64()
}

/// [`to_json_f64`] with a zero fallback, for response fields that are typed as a
/// non-optional number.
pub fn to_json_f64_or_zero(value: Decimal) -> f64 {
    to_json_f64(value).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_scale_values_survive_the_conversion() {
        let from_pg = "1000.000000000000000000000000000000"
            .parse::<Decimal>()
            .unwrap();
        assert_eq!(to_json_f64(from_pg), Some(1000.0));

        let cents = "0.002880000000000000000000000000"
            .parse::<Decimal>()
            .unwrap();
        assert_eq!(to_json_f64(cents), Some(0.00288));

        let tiny = "0.000002000000000000000000000000"
            .parse::<Decimal>()
            .unwrap();
        assert_eq!(to_json_f64(tiny), Some(0.000002));
    }
}
