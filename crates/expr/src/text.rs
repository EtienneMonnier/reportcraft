//! Numbers and dates as text in the report's language: `&` and `CONCAT` write numbers with the
//! locale's decimal separator, and `TEXT(value, code)` follows an Excel format code.
//!
//! Format codes are written as in English Excel, whatever the report's language: `.` marks the
//! decimals and `,` the thousands (`#,##0.00`), and the result uses the locale's own separators
//! and day and month names (`TEXT(1234.5, "#,##0.00")` is `1 234,50` in French). Supported: digit
//! placeholders `0` and `#`, decimals, grouping, `%`, up to three sections (`pos;neg;zero`), text
//! in quotes or after `\`, `@` for the value as text, and date codes (`yyyy`, `yy`, `mmmm`,
//! `mmm`, `mm`, `m`, `dddd`, `ddd`, `dd`, `d`, `hh`, `h`, `mm` after hours, `ss`, `AM/PM`).
//! Anything else gives `#VALUE!`.

use crate::value::{ExprError, R, Val, parse_number, significant, write_number};

/// How numbers and dates become text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Locale {
    pub decimal: char,
    pub group: char,
    /// Day and month names (`dddd`, `mmmm`, `AM/PM`).
    pub names: chrono::Locale,
}

impl Default for Locale {
    fn default() -> Self {
        Locale { decimal: '.', group: ',', names: chrono::Locale::en_US }
    }
}

impl Locale {
    /// A value as text: numbers in their shortest form with the locale's decimal separator (no
    /// grouping, as Excel's General format), everything else as `Val`'s own text.
    pub fn text(&self, v: &Val) -> R<String> {
        match v {
            Val::Number(n) => Ok(write_number(*n).replace('.', &self.decimal.to_string())),
            other => other.to_text(),
        }
    }

    /// Text read as a number, accepting the locale's decimal separator and grouping too.
    pub fn parse_number(&self, t: &str) -> Option<f64> {
        parse_number(t).or_else(|| {
            if self.decimal == '.' {
                return None;
            }
            let plain: String = t.trim().chars().filter(|c| *c != self.group && *c != '\u{a0}' && *c != '\u{202f}').collect();
            parse_number(&plain.replace(self.decimal, "."))
        })
    }
}

/// One part of a format code.
#[derive(Debug, Clone, PartialEq)]
enum Part {
    Literal(String),
    /// The digits: `0` and `#` before and after the decimal point, and whether to group.
    Number {
        int_zeros: usize,
        dec_zeros: usize,
        dec_hashes: usize,
        grouping: bool,
    },
    Percent,
    /// `@`: the value as text.
    TextValue,
    /// A date or time element as a `strftime` directive.
    Date(&'static str),
}

/// `TEXT(value, code)`.
pub(crate) fn text(value: &Val, code: &str, locale: &Locale) -> R<Val> {
    value.err()?;
    if code.len() > 255 {
        return Err(ExprError::Value);
    }
    let sections = split_sections(code);
    let number = match value {
        Val::Number(n) => Some(*n),
        Val::Bool(_) | Val::Blank => None,
        Val::Text(t) if !value.is_date_like() => locale.parse_number(t),
        _ => None,
    };
    // Pick the section: negative numbers use the second (without their sign), zero the third.
    let (section, n) = match number {
        Some(n) if n < 0.0 && sections.len() > 1 => (sections.get(1), n.abs()),
        Some(n) if n == 0.0 && sections.len() > 2 => (sections.get(2), n),
        Some(n) => (sections.first(), n),
        None => (sections.first(), 0.0),
    };
    let parts = parse(section.map_or("", String::as_str))?;
    let has_date = parts.iter().any(|p| matches!(p, Part::Date(_)));
    let has_number = parts.iter().any(|p| matches!(p, Part::Number { .. }));
    if has_date {
        if has_number {
            return Err(ExprError::Value);
        }
        let dt = value.as_datetime().ok_or(ExprError::Value)?;
        let pattern: String = parts
            .iter()
            .map(|p| match p {
                Part::Date(d) => (*d).to_owned(),
                Part::Literal(l) => l.replace('%', "%%"),
                Part::Percent => "%%".to_owned(),
                Part::TextValue | Part::Number { .. } => String::new(),
            })
            .collect();
        let items: Vec<chrono::format::Item<'_>> = chrono::format::StrftimeItems::new(&pattern).collect();
        if items.iter().any(|i| matches!(i, chrono::format::Item::Error)) {
            return Err(ExprError::Value);
        }
        return Ok(Val::Text(dt.and_utc().format_localized_with_items(items.into_iter(), locale.names).to_string()));
    }
    let percent = parts.iter().filter(|p| matches!(p, Part::Percent)).count();
    let mut n = n;
    if has_number && number.is_none() {
        // A number format on text that isn't a number leaves the text, as Excel does.
        return Ok(Val::Text(locale.text(value)?));
    }
    for _ in 0..percent.min(3) {
        n *= 100.0;
    }
    let mut out = String::new();
    // The sign is written once, before everything, unless a section for negatives was used.
    if n < 0.0 && sections.len() < 2 && has_number {
        out.push('-');
        n = n.abs();
    }
    for part in &parts {
        match part {
            Part::Literal(l) => out.push_str(l),
            Part::Percent => out.push('%'),
            Part::TextValue => out.push_str(&locale.text(value)?),
            Part::Number { int_zeros, dec_zeros, dec_hashes, grouping } => {
                out.push_str(&digits(n, *int_zeros, *dec_zeros, *dec_hashes, *grouping, locale));
            }
            Part::Date(_) => {}
        }
    }
    if out.len() > crate::MAX_TEXT {
        return Err(ExprError::Value);
    }
    Ok(Val::Text(out))
}

/// `n` (not negative) with the given placeholders.
fn digits(n: f64, int_zeros: usize, dec_zeros: usize, dec_hashes: usize, grouping: bool, locale: &Locale) -> String {
    let places = (dec_zeros + dec_hashes).min(15);
    let scale = 10f64.powi(i32::try_from(places).unwrap_or(15));
    let rounded = (significant(n * scale)).round() / scale;
    let fixed = format!("{rounded:.places$}");
    let (int_part, frac_part) = fixed.split_once('.').unwrap_or((fixed.as_str(), ""));
    let int_part = int_part.trim_start_matches('0');
    let int_part = format!("{int_part:0>int_zeros$}");
    let mut out = String::new();
    let len = int_part.chars().count();
    for (i, c) in int_part.chars().enumerate() {
        if grouping && i > 0 && (len - i) % 3 == 0 {
            out.push(locale.group);
        }
        out.push(c);
    }
    let mut frac: String = frac_part.to_owned();
    while frac.len() > dec_zeros && frac.ends_with('0') {
        frac.pop();
    }
    if !frac.is_empty() {
        out.push(locale.decimal);
        out.push_str(&frac);
    }
    out
}

/// The `;`-separated sections, quotes respected.
fn split_sections(code: &str) -> Vec<String> {
    let mut sections = vec![String::new()];
    let mut quoted = false;
    let mut escaped = false;
    for c in code.chars() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            quoted = !quoted;
        } else if c == ';' && !quoted {
            sections.push(String::new());
            continue;
        }
        if let Some(last) = sections.last_mut() {
            last.push(c);
        }
    }
    sections.truncate(3);
    sections
}

/// A section of a format code as parts.
fn parse(code: &str) -> R<Vec<Part>> {
    let chars: Vec<char> = code.chars().collect();
    let mut parts: Vec<Part> = Vec::new();
    let mut literal = String::new();
    let flush = |literal: &mut String, parts: &mut Vec<Part>| {
        if !literal.is_empty() {
            parts.push(Part::Literal(std::mem::take(literal)));
        }
    };
    let twelve_hours = code.to_ascii_uppercase().contains("AM/PM");
    let mut i = 0;
    let mut after_hours = false;
    while let Some(&c) = chars.get(i) {
        let run = chars.iter().skip(i).take_while(|&&x| x.eq_ignore_ascii_case(&c)).count();
        match c {
            '"' => {
                let close = chars.iter().skip(i + 1).position(|&x| x == '"').ok_or(ExprError::Value)?;
                literal.extend(chars.iter().skip(i + 1).take(close));
                i += close + 2;
                continue;
            }
            '\\' => {
                literal.extend(chars.get(i + 1));
                i += 2;
                continue;
            }
            '0' | '#' | '.' | ',' => {
                flush(&mut literal, &mut parts);
                let mut j = i;
                let (mut int_zeros, mut dec_zeros, mut dec_hashes, mut grouping, mut decimals) = (0, 0, 0, false, false);
                while let Some(&d) = chars.get(j) {
                    match d {
                        '0' if decimals => dec_zeros += 1,
                        '#' if decimals => dec_hashes += 1,
                        '0' => int_zeros += 1,
                        '#' => {}
                        ',' if !decimals => grouping = true,
                        '.' if !decimals => decimals = true,
                        _ => break,
                    }
                    j += 1;
                }
                if parts.iter().any(|p| matches!(p, Part::Number { .. })) {
                    return Err(ExprError::Value);
                }
                parts.push(Part::Number { int_zeros, dec_zeros, dec_hashes, grouping });
                i = j;
                continue;
            }
            '%' => {
                flush(&mut literal, &mut parts);
                parts.push(Part::Percent);
            }
            '@' => {
                flush(&mut literal, &mut parts);
                parts.push(Part::TextValue);
            }
            'y' | 'Y' => {
                flush(&mut literal, &mut parts);
                parts.push(Part::Date(if run >= 3 { "%Y" } else { "%y" }));
                i += run;
                continue;
            }
            'd' | 'D' => {
                flush(&mut literal, &mut parts);
                parts.push(Part::Date(match run {
                    1 => "%-d",
                    2 => "%d",
                    3 => "%a",
                    _ => "%A",
                }));
                i += run;
                continue;
            }
            'h' | 'H' => {
                flush(&mut literal, &mut parts);
                parts.push(Part::Date(match (run, twelve_hours) {
                    (1, false) => "%-H",
                    (_, false) => "%H",
                    (1, true) => "%-I",
                    (_, true) => "%I",
                }));
                after_hours = true;
                i += run;
                continue;
            }
            's' | 'S' => {
                flush(&mut literal, &mut parts);
                parts.push(Part::Date(if run == 1 { "%-S" } else { "%S" }));
                // `mm` just before seconds means minutes.
                if let Some(Part::Date(prev)) = parts.iter_mut().rev().nth(1)
                    && (*prev == "%m" || *prev == "%-m")
                {
                    *prev = if *prev == "%m" { "%M" } else { "%-M" };
                }
                i += run;
                continue;
            }
            'm' | 'M' => {
                flush(&mut literal, &mut parts);
                let minutes = after_hours && run <= 2;
                parts.push(Part::Date(match (run, minutes) {
                    (1, true) => "%-M",
                    (_, true) => "%M",
                    (1, false) => "%-m",
                    (2, false) => "%m",
                    (3, false) => "%b",
                    _ => "%B",
                }));
                after_hours = false;
                i += run;
                continue;
            }
            'a' | 'A' if chars.get(i..i + 5).is_some_and(|w| w.iter().collect::<String>().eq_ignore_ascii_case("AM/PM")) => {
                flush(&mut literal, &mut parts);
                parts.push(Part::Date("%p"));
                i += 5;
                continue;
            }
            ' ' | '-' | '/' | ':' | '(' | ')' | '+' | '$' | '€' | '£' | '¥' | '\u{a0}' => literal.push(c),
            _ => return Err(ExprError::Value),
        }
        i += 1;
    }
    flush(&mut literal, &mut parts);
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn fr() -> Locale {
        Locale { decimal: ',', group: '\u{202f}', names: chrono::Locale::fr_FR }
    }

    fn t(v: Val, code: &str, locale: &Locale) -> String {
        text(&v, code, locale).map_or_else(|e| e.to_string(), |v| v.to_string())
    }

    #[test]
    fn numbers_follow_the_code_and_the_locale() {
        let en = Locale::default();
        let n = |x: f64, code: &str| t(Val::Number(x), code, &en);
        assert_eq!(n(1234.5, "#,##0.00"), "1,234.50");
        assert_eq!(t(Val::Number(1234.5), "#,##0.00", &fr()), "1\u{202f}234,50");
        assert_eq!(n(0.5, "0%"), "50%");
        assert_eq!(n(0.1234, "0.0%"), "12.3%");
        assert_eq!(n(3.0, "000"), "003");
        assert_eq!(n(2.675, "0.00"), "2.68");
        assert_eq!(n(1.5, "0.##"), "1.5");
        assert_eq!(n(2.0, "0.##"), "2");
        assert_eq!(n(-5.0, "0.0"), "-5.0");
        assert_eq!(n(-5.0, "0.0;(0.0)"), "(5.0)");
        assert_eq!(n(0.0, "0;-0;\"nil\""), "nil");
        assert_eq!(n(12.0, "\"Total: \"0\" units\""), "Total: 12 units");
        assert_eq!(n(12.0, "0 €"), "12 €");
        assert_eq!(t(Val::Text("12.5".into()), "0.0", &en), "12.5", "numeric text is a number");
        assert_eq!(t(Val::Text("n/a".into()), "0.0", &en), "n/a", "other text stays");
        assert_eq!(t(Val::Text("abc".into()), "[@]", &en), "#VALUE!", "unsupported code");
        assert_eq!(t(Val::Text("abc".into()), "\"<\"@\">\"", &en), "<abc>");
    }

    #[test]
    fn dates_follow_the_code_and_the_language() {
        let d = Val::DateTime(NaiveDate::from_ymd_opt(2026, 11, 11).and_then(|d| d.and_hms_opt(15, 7, 9)).unwrap());
        let en = Locale::default();
        assert_eq!(t(d.clone(), "dd/mm/yyyy", &en), "11/11/2026");
        assert_eq!(t(d.clone(), "dddd d mmmm yyyy", &fr()), "mercredi 11 novembre 2026");
        assert_eq!(t(d.clone(), "ddd mmm yy", &en), "Wed Nov 26");
        assert_eq!(t(d.clone(), "hh:mm:ss", &en), "15:07:09");
        assert_eq!(t(d.clone(), "h:mm AM/PM", &en), "3:07 PM");
        assert_eq!(t(Val::Text("2026-01-05".into()), "d/m", &en), "5/1");
        assert_eq!(t(Val::Number(45000.0), "dd/mm/yyyy", &en), "#VALUE!", "no serial numbers");
        assert_eq!(t(d, "yyyy 0", &en), "#VALUE!", "a date and a number in one code");
    }

    #[test]
    fn locale_text_and_parsing() {
        assert_eq!(fr().text(&Val::Number(10.8)).unwrap(), "10,8");
        assert_eq!(Locale::default().text(&Val::Number(10.8)).unwrap(), "10.8");
        assert_eq!(fr().parse_number("1\u{202f}234,5"), Some(1234.5));
        assert_eq!(fr().parse_number("10.8"), Some(10.8), "a plain number still reads");
        assert_eq!(Locale::default().parse_number("10,8"), None);
    }
}
