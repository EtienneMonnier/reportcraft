//! ReportCraft core (L0): the values that flow from data sources into reports, and the shared
//! error type. No I/O, no dependencies on other ReportCraft crates, wasm-safe.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Hard caps on input-sized work. Input-derived numbers are hostile (AGENTS.md).
pub mod limits {
    /// Most rows one data source may hand to a report.
    pub const MAX_ROWS: usize = 1_000_000;
    /// Most columns in one row set.
    pub const MAX_COLUMNS: usize = 1_024;
    /// Most pages one render may produce.
    pub const MAX_PAGES: usize = 10_000;
    /// Most nested groups in one report.
    pub const MAX_GROUPS: usize = 16;
    /// Most elements in one band.
    pub const MAX_ELEMENTS_PER_BAND: usize = 512;
    /// Default for the memory a data source's rows may take (an estimate, see `Value::size`).
    pub const DEFAULT_DATA_BYTES: usize = 1 << 30;
    /// Most memory a caller may allow a data source's rows to take.
    pub const MAX_DATA_BYTES: usize = 16 << 30;
}

/// Stops long work (a query, a render): a flag another thread raises, plus an optional deadline.
/// Clones share the flag. Work checks it between steps and stops with `Error::Cancelled` or, past
/// the deadline, `Error::TooLarge`.
#[derive(Debug, Clone, Default)]
pub struct Cancel {
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    deadline: Option<std::time::Instant>,
}

impl Cancel {
    /// Asks the work using this flag (or a clone of it) to stop.
    pub fn cancel(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Lowers the flag, ready for the next piece of work.
    pub fn reset(&self) {
        self.flag.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The same flag, with a deadline `limit` from now. Native only: the web build has no clock.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_time_limit(&self, limit: std::time::Duration) -> Self {
        #[allow(clippy::disallowed_methods)]
        let now = std::time::Instant::now();
        Self { flag: self.flag.clone(), deadline: now.checked_add(limit) }
    }

    pub fn is_past_deadline(&self) -> bool {
        // Only a native caller can have set a deadline, so the clock is never read on wasm.
        #[allow(clippy::disallowed_methods)]
        self.deadline.is_some_and(|d| std::time::Instant::now() >= d)
    }

    /// `Err` once the work should stop. `what` names it in the message ("the render").
    pub fn check(&self, what: &str) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::Cancelled(format!("{what} was stopped on request")))
        } else if self.is_past_deadline() {
            Err(Error::TooLarge(format!("{what} ran past its time limit and was stopped")))
        } else {
            Ok(())
        }
    }
}

/// One value in a row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(untagged)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Int(i64),
    Number(f64),
    Text(String),
}

impl Value {
    /// The value as a number, when it is one (or text that reads as one).
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Number(n) if n.is_finite() => Some(*n),
            Value::Text(t) => t.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
            Value::Bool(_) | Value::Null | Value::Number(_) => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// The memory the value takes, roughly: the enum plus any text.
    pub fn size(&self) -> usize {
        std::mem::size_of::<Value>() + if let Value::Text(t) = self { t.len() } else { 0 }
    }

    /// Total order used for sorting and group breaks: nulls first, then numbers, then text.
    pub fn sort_cmp(&self, other: &Value) -> Ordering {
        fn rank(v: &Value) -> u8 {
            match v {
                Value::Null => 0,
                Value::Bool(_) => 1,
                Value::Int(_) | Value::Number(_) => 2,
                Value::Text(_) => 3,
            }
        }
        match (self, other) {
            (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
            (Value::Text(a), Value::Text(b)) => a.cmp(b),
            (a, b) if rank(a) == 2 && rank(b) == 2 => {
                let (x, y) = (a.as_f64().unwrap_or(0.0), b.as_f64().unwrap_or(0.0));
                x.total_cmp(&y)
            }
            (a, b) => rank(a).cmp(&rank(b)),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => Ok(()),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Number(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    write!(f, "{n:.0}")
                } else {
                    write!(f, "{n}")
                }
            }
            Value::Text(t) => f.write_str(t),
        }
    }
}

/// The declared type of a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    #[default]
    Text,
    Integer,
    Number,
    Bool,
}

impl FieldType {
    /// Reads raw text as this type; text that doesn't fit stays text, so nothing is lost.
    pub fn parse(self, raw: &str) -> Value {
        let t = raw.trim();
        if t.is_empty() {
            return Value::Null;
        }
        match self {
            FieldType::Text => Value::Text(raw.to_owned()),
            FieldType::Integer => t.parse::<i64>().map(Value::Int).unwrap_or_else(|_| Value::Text(raw.to_owned())),
            FieldType::Number => t.parse::<f64>().ok().filter(|n| n.is_finite()).map(Value::Number).unwrap_or_else(|| Value::Text(raw.to_owned())),
            FieldType::Bool => match t.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" => Value::Bool(true),
                "false" | "0" | "no" => Value::Bool(false),
                _ => Value::Text(raw.to_owned()),
            },
        }
    }
}

/// A column of a row set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    #[serde(default)]
    pub field_type: FieldType,
}

/// Rows read from a data source, column order fixed by `columns`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RowSet {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
}

impl RowSet {
    /// Index of a column by name (case-insensitive).
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// The value of `column` in `row`, `Null` when either is out of range.
    pub fn value<'a>(&'a self, row: &'a [Value], column: usize) -> &'a Value {
        const NULL: &Value = &Value::Null;
        row.get(column).unwrap_or(NULL)
    }
}

/// The error every ReportCraft crate reports through `?`.
#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum Error {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("too large: {0}")]
    TooLarge(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("i/o: {0}")]
    Io(String),
    #[error("cancelled: {0}")]
    Cancelled(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_is_shared_by_clones_and_deadlines_stop_work() {
        let c = Cancel::default();
        let other = c.clone();
        assert!(c.check("the render").is_ok());
        other.cancel();
        assert!(matches!(c.check("the render"), Err(Error::Cancelled(_))));
        c.reset();
        assert!(!other.is_cancelled());
        let late = c.with_time_limit(std::time::Duration::ZERO);
        assert!(matches!(late.check("the query"), Err(Error::TooLarge(m)) if m.contains("time limit")));
        late.cancel();
        assert!(c.is_cancelled(), "a deadline keeps the shared flag");
    }

    #[test]
    fn numbers_sort_before_text_and_after_null() {
        let mut v = vec![Value::Text("a".into()), Value::Int(3), Value::Null, Value::Number(1.5)];
        v.sort_by(Value::sort_cmp);
        assert_eq!(v, vec![Value::Null, Value::Number(1.5), Value::Int(3), Value::Text("a".into())]);
    }

    #[test]
    fn parse_keeps_text_that_does_not_fit_the_type() {
        assert_eq!(FieldType::Integer.parse("12"), Value::Int(12));
        assert_eq!(FieldType::Integer.parse("12x"), Value::Text("12x".into()));
        assert_eq!(FieldType::Number.parse("NaN"), Value::Text("NaN".into()));
        assert_eq!(FieldType::Number.parse("  "), Value::Null);
    }

    #[test]
    fn value_out_of_range_is_null() {
        let rs = RowSet::default();
        assert!(rs.value(&[], 7).is_null());
    }
}
