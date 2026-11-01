//! Writes a value with an element's format and the report's locale.

use chrono::format::{Item, StrftimeItems};
use chrono::{DateTime, NaiveDate, NaiveDateTime};
use reportcraft_core::Value;
use reportcraft_model::{Format, LocaleStyle, MAX_DECIMALS};

/// The text shown for `value`. Without a format, or when the format doesn't apply to the value,
/// the value is written as it is.
pub fn write(value: &Value, format: Option<&Format>, locale: &LocaleStyle) -> String {
    let Some(format) = format else { return value.to_string() };
    if value.is_null() {
        return String::new();
    }
    match format {
        Format::Number { decimals, grouping } => match value.as_f64() {
            Some(n) => number(n, *decimals, *grouping, locale),
            None => value.to_string(),
        },
        Format::Currency { symbol, decimals, symbol_after } => match value.as_f64() {
            Some(n) => currency(n, symbol, *decimals, *symbol_after, locale),
            None => value.to_string(),
        },
        Format::Percent { decimals } => match value.as_f64().map(|n| n * 100.0).filter(|n| n.is_finite()) {
            Some(n) => format!("{}{}%", number(n, *decimals, true, locale), locale.unit_space),
            None => value.to_string(),
        },
        Format::Date { pattern } => match value {
            Value::Text(t) => date(t, pattern.as_deref().unwrap_or(locale.date_pattern)).unwrap_or_else(|| t.clone()),
            other => other.to_string(),
        },
    }
}

/// `n` rounded to `decimals`, with the locale's separators. Never writes `-0`.
fn number(n: f64, decimals: u8, grouping: bool, locale: &LocaleStyle) -> String {
    let decimals = usize::from(decimals.min(MAX_DECIMALS));
    let fixed = format!("{:.*}", decimals, n.abs());
    let negative = n < 0.0 && fixed.chars().any(|c| c.is_ascii_digit() && c != '0');
    let (int_part, frac_part) = fixed.split_once('.').unwrap_or((fixed.as_str(), ""));
    let mut out = String::with_capacity(fixed.len() + fixed.len() / 3 + 1);
    if negative {
        out.push('-');
    }
    let digits = int_part.len();
    for (i, c) in int_part.chars().enumerate() {
        if grouping && i > 0 && (digits - i) % 3 == 0 {
            out.push(locale.group);
        }
        out.push(c);
    }
    if !frac_part.is_empty() {
        out.push(locale.decimal);
        out.push_str(frac_part);
    }
    out
}

fn currency(n: f64, symbol: &str, decimals: u8, symbol_after: bool, locale: &LocaleStyle) -> String {
    let amount = number(n.abs(), decimals, true, locale);
    let negative = n < 0.0 && amount.chars().any(|c| c.is_ascii_digit() && c != '0');
    let sign = if negative { "-" } else { "" };
    if symbol_after {
        let space = if locale.unit_space.is_empty() { "\u{a0}" } else { locale.unit_space };
        format!("{sign}{amount}{space}{symbol}")
    } else {
        format!("{sign}{symbol}{amount}")
    }
}

/// A date or date-time written as text by a data source (ISO 8601, SQL, RFC 3339), rewritten
/// with `pattern`. `None` when the text isn't a date or the pattern isn't valid.
fn date(text: &str, pattern: &str) -> Option<String> {
    let items: Vec<Item<'_>> = StrftimeItems::new(pattern).collect();
    if items.iter().any(|i| matches!(i, Item::Error)) {
        return None;
    }
    let t = text.trim();
    let parsed = DateTime::parse_from_rfc3339(t)
        .map(|d| d.naive_local())
        .ok()
        .or_else(|| {
            ["%Y-%m-%d %H:%M:%S%.f%#z", "%Y-%m-%d %H:%M:%S%#z"].iter().find_map(|f| DateTime::parse_from_str(t, f).ok().map(|d| d.naive_local()))
        })
        .or_else(|| ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M"].iter().find_map(|f| NaiveDateTime::parse_from_str(t, f).ok()))
        .or_else(|| NaiveDate::parse_from_str(t, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0)))?;
    let mut out = String::new();
    std::fmt::Write::write_fmt(&mut out, format_args!("{}", parsed.format_with_items(items.into_iter()))).ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reportcraft_model::locale_or_default;

    fn num(n: f64, d: u8, tag: &str) -> String {
        write(&Value::Number(n), Some(&Format::Number { decimals: d, grouping: true }), &locale_or_default(tag))
    }

    #[test]
    fn numbers_follow_the_locale() {
        assert_eq!(num(1234567.891, 2, "en"), "1,234,567.89");
        assert_eq!(num(1234567.891, 2, "fr"), "1\u{202f}234\u{202f}567,89");
        assert_eq!(num(1234567.891, 0, "de"), "1.234.568");
        assert_eq!(num(-1234.5, 1, "de-CH"), "-1\u{2019}234.5");
        assert_eq!(num(999.0, 0, "en"), "999");
        assert_eq!(num(-0.001, 2, "en"), "0.00", "no negative zero");
        assert_eq!(write(&Value::Int(1000), Some(&Format::Number { decimals: 0, grouping: false }), &locale_or_default("en")), "1000");
        assert_eq!(num(1e300, 0, "en").len(), 401, "huge numbers are written in full, not panicking");
    }

    #[test]
    fn currencies_and_percentages() {
        let eur = Format::Currency { symbol: "€".into(), decimals: 2, symbol_after: true };
        let usd = Format::Currency { symbol: "$".into(), decimals: 2, symbol_after: false };
        assert_eq!(write(&Value::Number(1260.5), Some(&eur), &locale_or_default("fr")), "1\u{202f}260,50\u{a0}€");
        assert_eq!(write(&Value::Number(-5.0), Some(&usd), &locale_or_default("en-US")), "-$5.00");
        assert_eq!(write(&Value::Text("12.5".into()), Some(&usd), &locale_or_default("en")), "$12.50", "numeric text");
        let pct = Format::Percent { decimals: 1 };
        assert_eq!(write(&Value::Number(0.125), Some(&pct), &locale_or_default("en")), "12.5%");
        assert_eq!(write(&Value::Number(0.125), Some(&pct), &locale_or_default("fr")), "12,5\u{a0}%");
    }

    #[test]
    fn dates_from_iso_sql_and_rfc3339() {
        let d = |text: &str, pattern: Option<&str>, tag: &str| {
            write(&Value::Text(text.into()), Some(&Format::Date { pattern: pattern.map(str::to_owned) }), &locale_or_default(tag))
        };
        assert_eq!(d("2026-10-01", None, "fr"), "01/10/2026");
        assert_eq!(d("2026-10-01", None, "en-US"), "10/01/2026");
        assert_eq!(d("2026-10-01 14:05:00", Some("%d.%m.%Y %H:%M"), "de"), "01.10.2026 14:05");
        assert_eq!(d("2026-10-01 14:05:00.123+02", Some("%H:%M"), "en"), "14:05");
        assert_eq!(d("2026-10-01T14:05:00Z", None, "en"), "2026-10-01");
        assert_eq!(d("not a date", None, "en"), "not a date");
        assert_eq!(d("2026-10-01", Some("%Q"), "en"), "2026-10-01", "a bad pattern leaves the text");
    }

    #[test]
    fn values_the_format_does_not_fit_are_written_as_they_are() {
        let n = Format::Number { decimals: 2, grouping: true };
        let en = locale_or_default("en");
        assert_eq!(write(&Value::Text("n/a".into()), Some(&n), &en), "n/a");
        assert_eq!(write(&Value::Null, Some(&n), &en), "");
        assert_eq!(write(&Value::Int(3), Some(&Format::Date { pattern: None }), &en), "3");
        assert_eq!(write(&Value::Number(2.5), None, &en), "2.5");
    }
}
