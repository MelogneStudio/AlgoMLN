/// Convert a rupee-denominated market value to integer paise using ties-to-even.
///
/// Routes through the same integer half-even path as `decimal_scaled` so that a
/// price carried as `f64` (candle close, config cash) rounds identically to the
/// decimal literals from the DSL. The `f64` is rendered to a fixed 9-fraction-digit
/// decimal string (enough to capture sub-paise without the `value * 100.0`
/// representation drift that made the old float path tie unstably), then rounded to
/// paise in `i128`.
pub fn rupees_to_paise(value: f64) -> i64 {
    if !value.is_finite() {
        return 0;
    }
    let scaled = decimal_scaled(&format!("{value:.9}"), 100)
        .unwrap_or_else(|| (value * 100.0).clamp(i64::MIN as f64, i64::MAX as f64) as i128);
    scaled.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

pub fn paise_to_rupees(value: i64) -> f64 {
    value as f64 / 100.0
}

pub(crate) fn round_half_even_ratio(numerator: i128, denominator: i128) -> i128 {
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    let twice = remainder.abs() * 2;
    if twice > denominator.abs() || (twice == denominator.abs() && quotient % 2 != 0) {
        quotient + numerator.signum() * denominator.signum()
    } else {
        quotient
    }
}

pub(crate) fn decimal_scaled(value: &str, scale: i128) -> Option<i128> {
    let negative = value.starts_with('-');
    let unsigned = value.trim_start_matches(['-', '+']);
    let mut parts = unsigned.split('.');
    let whole: i128 = parts.next()?.parse().ok()?;
    let fraction = parts.next().unwrap_or("");
    if fraction.len() > 9 || !fraction.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let denominator = 10i128.checked_pow(fraction.len() as u32)?;
    let fractional = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<i128>().ok()?
    };
    let scaled = whole
        .checked_mul(scale)?
        .checked_add(round_half_even_ratio(
            fractional.checked_mul(scale)?,
            denominator,
        ))?;
    Some(if negative { -scaled } else { scaled })
}

pub(crate) fn decimal_rupees_to_paise(value: &str) -> Option<i64> {
    i64::try_from(decimal_scaled(value, 100)?).ok()
}
