//! Formula values and the conversions between them.

use std::cmp::Ordering;
use std::fmt;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};

/// An error value, written as Excel writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExprError {
    /// Division by zero, or an average of nothing.
    Div0,
    /// A value of the wrong kind (text where a number is needed…), or a result too large.
    Value,
    /// An unknown field.
    Name,
    /// A number out of range (not finite, a negative square root, an impossible date).
    Num,
    /// No value available (`TODAY()` when the render has no clock, `SWITCH` without a match).
    Na,
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ExprError::Div0 => "#DIV/0!",
            ExprError::Value => "#VALUE!",
            ExprError::Name => "#NAME?",
            ExprError::Num => "#NUM!",
            ExprError::Na => "#N/A",
        })
    }
}

/// One formula value.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Val {
    /// An empty field or a missing value.
    #[default]
    Blank,
    Bool(bool),
    /// Always finite: a calculation that would give infinity or NaN gives `#NUM!`.
    Number(f64),
    Text(String),
    Date(NaiveDate),
    DateTime(NaiveDateTime),
    Error(ExprError),
}

pub(crate) type R<T> = Result<T, ExprError>;

impl Val {
    /// A number, or `#NUM!` when it isn't finite.
    pub fn number(n: f64) -> Val {
        if n.is_finite() { Val::Number(n) } else { Val::Error(ExprError::Num) }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Val::Error(_))
    }

    pub(crate) fn err(&self) -> R<()> {
        if let Val::Error(e) = self { Err(*e) } else { Ok(()) }
    }

    /// The value in arithmetic: a blank is 0, `TRUE` is 1, text must read as a number.
    pub(crate) fn to_number(&self) -> R<f64> {
        match self {
            Val::Blank => Ok(0.0),
            Val::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Val::Number(n) => Ok(*n),
            Val::Text(t) => parse_number(t).ok_or(ExprError::Value),
            Val::Date(_) | Val::DateTime(_) => Err(ExprError::Value),
            Val::Error(e) => Err(*e),
        }
    }

    /// The value as text: numbers in their shortest form, dates in ISO 8601.
    pub(crate) fn to_text(&self) -> R<String> {
        match self {
            Val::Error(e) => Err(*e),
            other => Ok(other.to_string()),
        }
    }

    pub(crate) fn to_bool(&self) -> R<bool> {
        match self {
            Val::Blank => Ok(false),
            Val::Bool(b) => Ok(*b),
            Val::Number(n) => Ok(*n != 0.0),
            Val::Text(t) if t.eq_ignore_ascii_case("true") => Ok(true),
            Val::Text(t) if t.eq_ignore_ascii_case("false") => Ok(false),
            Val::Error(e) => Err(*e),
            Val::Text(_) | Val::Date(_) | Val::DateTime(_) => Err(ExprError::Value),
        }
    }

    /// The value as a date-time, when it is a date or text holding one.
    pub(crate) fn as_datetime(&self) -> Option<NaiveDateTime> {
        match self {
            Val::Date(d) => Some(d.and_time(NaiveTime::MIN)),
            Val::DateTime(dt) => Some(*dt),
            Val::Text(t) => parse_datetime(t),
            _ => None,
        }
    }

    /// Whether the value is a date, or text holding one (and not a number).
    pub(crate) fn is_date_like(&self) -> bool {
        match self {
            Val::Date(_) | Val::DateTime(_) => true,
            Val::Text(t) => parse_number(t).is_none() && parse_datetime(t).is_some(),
            _ => false,
        }
    }

    /// The value as a date: dates, date-times (their day) and text holding one.
    pub(crate) fn to_date(&self) -> R<NaiveDate> {
        self.err()?;
        self.as_datetime().map(|dt| dt.date()).ok_or(ExprError::Value)
    }
}

/// Text read as a number: trimmed, `.` as the decimal separator, finite.
pub(crate) fn parse_number(t: &str) -> Option<f64> {
    let t = t.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("nan") || t.to_ascii_lowercase().contains("inf") {
        return None;
    }
    t.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Text read as a date or date-time: ISO 8601 / SQL (`2026-10-31`, `2026-10-31 14:05:09`,
/// `2026-10-31T14:05`) or RFC 3339 (the offset is dropped: the local time is kept).
pub(crate) fn parse_datetime(t: &str) -> Option<NaiveDateTime> {
    let t = t.trim();
    if t.len() < 10 || t.len() > 40 {
        return None;
    }
    if let Ok(d) = NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        return Some(d.and_time(NaiveTime::MIN));
    }
    for pattern in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(t, pattern) {
            return Some(dt);
        }
    }
    chrono::DateTime::parse_from_rfc3339(t).ok().map(|dt| dt.naive_local())
}

/// A number written in its shortest form, at most 15 significant digits as Excel shows them,
/// so `0.1 + 0.2` reads `0.3`.
pub(crate) fn write_number(n: f64) -> String {
    let n = significant(n);
    if n == 0.0 {
        return "0".to_owned();
    }
    if n.fract() == 0.0 && n.abs() < 1e15 { format!("{n:.0}") } else { format!("{n}") }
}

/// `n` rounded to 15 significant digits (what Excel keeps).
pub fn significant(n: f64) -> f64 {
    if !n.is_finite() || n == 0.0 {
        return n;
    }
    format!("{n:.14e}").parse::<f64>().unwrap_or(n)
}

impl fmt::Display for Val {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Val::Blank => Ok(()),
            Val::Bool(true) => f.write_str("TRUE"),
            Val::Bool(false) => f.write_str("FALSE"),
            Val::Number(n) => f.write_str(&write_number(*n)),
            Val::Text(t) => f.write_str(t),
            Val::Date(d) => write!(f, "{}", d.format("%Y-%m-%d")),
            Val::DateTime(dt) => write!(f, "{}", dt.format("%Y-%m-%d %H:%M:%S")),
            Val::Error(e) => write!(f, "{e}"),
        }
    }
}

/// Excel's comparison: numbers < text < booleans; text ignores case; a blank compares as 0, `""`
/// or `FALSE` depending on the other side. Dates compare with dates, and with text holding one.
pub(crate) fn compare(a: &Val, b: &Val) -> R<Ordering> {
    a.err()?;
    b.err()?;
    let blank_like = |other: &Val| match other {
        Val::Number(_) => Val::Number(0.0),
        Val::Bool(_) => Val::Bool(false),
        _ => Val::Text(String::new()),
    };
    match (a, b) {
        (Val::Blank, Val::Blank) => Ok(Ordering::Equal),
        (Val::Blank, other) if !matches!(other, Val::Date(_) | Val::DateTime(_)) => compare(&blank_like(other), other),
        (other, Val::Blank) if !matches!(other, Val::Date(_) | Val::DateTime(_)) => compare(other, &blank_like(other)),
        // A blank date is earlier than any date (Excel's 0).
        (Val::Blank, _) => Ok(Ordering::Less),
        (_, Val::Blank) => Ok(Ordering::Greater),
        (Val::Date(_) | Val::DateTime(_), _) | (_, Val::Date(_) | Val::DateTime(_)) => match (a.as_datetime(), b.as_datetime()) {
            (Some(x), Some(y)) => Ok(x.cmp(&y)),
            _ => Err(ExprError::Value),
        },
        (Val::Number(x), Val::Number(y)) => Ok(x.total_cmp(y)),
        (Val::Text(x), Val::Text(y)) => Ok(x.to_lowercase().cmp(&y.to_lowercase())),
        (Val::Bool(x), Val::Bool(y)) => Ok(x.cmp(y)),
        _ => Ok(rank(a).cmp(&rank(b))),
    }
}

fn rank(v: &Val) -> u8 {
    match v {
        Val::Number(_) => 0,
        Val::Text(_) => 1,
        _ => 2,
    }
}
