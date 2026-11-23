//! Writes a value with an element's format and the report's locale.

use chrono::format::{Item, StrftimeItems};
use chrono::{DateTime, NaiveDate, NaiveDateTime};
use reportcraft_core::Value;
use reportcraft_model::{Format, LocaleStyle, MAX_DECIMALS, TextCase};

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
            Value::Text(t) => {
                let pattern = pattern.as_deref().unwrap_or(locale.date_pattern);
                // A date range parameter (`from..to`) writes both ends.
                let range =
                    t.split_once("..").and_then(|(a, b)| Some(format!("{} – {}", date(a, pattern, locale.names)?, date(b, pattern, locale.names)?)));
                range.or_else(|| date(t, pattern, locale.names)).unwrap_or_else(|| t.clone())
            }
            other => other.to_string(),
        },
        Format::Boolean { true_text, false_text } => match truth(value) {
            Some(true) => true_text.clone().unwrap_or_else(|| locale.yes.to_owned()),
            Some(false) => false_text.clone().unwrap_or_else(|| locale.no.to_owned()),
            None => value.to_string(),
        },
        Format::Text { case, prefix, suffix } => {
            let text = value.to_string();
            if text.trim().is_empty() {
                return String::new();
            }
            let text = match case {
                None => text,
                Some(TextCase::Upper) => text.to_uppercase(),
                Some(TextCase::Lower) => text.to_lowercase(),
                Some(TextCase::Title) => title_case(&text),
            };
            format!("{prefix}{text}{suffix}")
        }
    }
}

/// A value read as true or false: a boolean, 1 or 0, or the text `true`/`false` (any case).
fn truth(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::Int(1) => Some(true),
        Value::Int(0) => Some(false),
        Value::Number(n) if *n == 1.0 => Some(true),
        Value::Number(n) if *n == 0.0 => Some(false),
        Value::Text(t) if t.trim().eq_ignore_ascii_case("true") => Some(true),
        Value::Text(t) if t.trim().eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

/// Each word's first letter in capitals and the others small: a letter after a letter is small
/// (`jean-luc o'neil` → `Jean-Luc O'Neil`), as the `PROPER` formula writes it.
fn title_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut after_letter = false;
    for c in text.chars() {
        if after_letter {
            out.extend(c.to_lowercase());
        } else {
            out.extend(c.to_uppercase());
        }
        after_letter = c.is_alphabetic();
    }
    out
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
/// with `pattern`, day and month names in `names`' language. `None` when the text isn't a date
/// or the pattern isn't valid.
fn date(text: &str, pattern: &str, names: chrono::Locale) -> Option<String> {
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
    // The time is the one written in the data: it is formatted as is, not converted.
    std::fmt::Write::write_fmt(&mut out, format_args!("{}", parsed.and_utc().format_localized_with_items(items.into_iter(), names))).ok()?;
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
        assert_eq!(d("2026-10-01..2026-10-31", None, "fr"), "01/10/2026 – 31/10/2026");
        assert_eq!(d("2026-10-01", Some("%Q"), "en"), "2026-10-01", "a bad pattern leaves the text");
    }

    #[test]
    fn day_and_month_names_follow_the_report_language() {
        let d = |pattern: &str, tag: &str| {
            write(&Value::Text("2026-11-11 15:30:00".into()), Some(&Format::Date { pattern: Some(pattern.into()) }), &locale_or_default(tag))
        };
        let long = "%A %-d %B %Y";
        assert_eq!(d(long, "en"), "Wednesday 11 November 2026");
        assert_eq!(d(long, "fr"), "mercredi 11 novembre 2026");
        assert_eq!(d(long, "fr-CH"), "mercredi 11 novembre 2026");
        assert_eq!(d(long, "de"), "Mittwoch 11 November 2026");
        assert_eq!(d(long, "es"), "miércoles 11 noviembre 2026");
        assert_eq!(d(long, "it"), "mercoledì 11 novembre 2026");
        assert_eq!(d(long, "nl"), "woensdag 11 november 2026");
        assert_eq!(d(long, "pt"), "quarta 11 novembro 2026");
        assert_eq!(d("%a %b", "de"), "Mi Nov");
        assert_eq!(d("%B", "xx-YY"), "November", "an unknown locale writes English");
        assert_eq!(d("%I:%M %p", "en-US"), "03:30 PM");
    }

    #[test]
    fn yes_or_no_in_the_report_language_or_the_texts_given() {
        let yes_no = Format::Boolean { true_text: None, false_text: None };
        let w = |v: Value, f: &Format, tag: &str| write(&v, Some(f), &locale_or_default(tag));
        assert_eq!(w(Value::Bool(true), &yes_no, "en"), "Yes");
        assert_eq!(w(Value::Bool(false), &yes_no, "fr"), "Non");
        assert_eq!(w(Value::Int(1), &yes_no, "de-CH"), "Ja");
        assert_eq!(w(Value::Number(0.0), &yes_no, "pt"), "Não");
        assert_eq!(w(Value::Text(" TRUE ".into()), &yes_no, "es"), "Sí");
        assert_eq!(w(Value::Int(2), &yes_no, "en"), "2", "neither true nor false");
        assert_eq!(w(Value::Text("maybe".into()), &yes_no, "en"), "maybe");
        assert_eq!(w(Value::Null, &yes_no, "en"), "");
        let ticks = Format::Boolean { true_text: Some("✓".into()), false_text: Some(String::new()) };
        assert_eq!(w(Value::Bool(true), &ticks, "fr"), "✓");
        assert_eq!(w(Value::Bool(false), &ticks, "fr"), "");
    }

    #[test]
    fn text_in_a_case_between_a_prefix_and_a_suffix() {
        let text = |case, prefix: &str, suffix: &str| Format::Text { case, prefix: prefix.into(), suffix: suffix.into() };
        let en = locale_or_default("en");
        assert_eq!(write(&Value::Text("lyon".into()), Some(&text(Some(TextCase::Upper), "", "")), &en), "LYON");
        assert_eq!(write(&Value::Text("ÉTÉ".into()), Some(&text(Some(TextCase::Lower), "", "")), &en), "été");
        assert_eq!(write(&Value::Text("jean-luc o'NEIL".into()), Some(&text(Some(TextCase::Title), "", "")), &en), "Jean-Luc O'Neil");
        assert_eq!(write(&Value::Int(1042), Some(&text(None, "Ref. ", "")), &en), "Ref. 1042");
        assert_eq!(write(&Value::Number(12.5), Some(&text(None, "", " kg")), &en), "12.5 kg");
        assert_eq!(write(&Value::Text("  ".into()), Some(&text(None, "Tel. ", "")), &en), "", "blank stays blank");
        assert_eq!(write(&Value::Null, Some(&text(None, "Tel. ", "")), &en), "");
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
