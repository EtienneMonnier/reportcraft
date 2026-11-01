//! Typed report parameters: what a report asks for when it runs (a period, a branch, a minimum
//! amount), how a supplied value is read and checked, and the resolved values that rendering
//! shows and queries bind.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::Diagnostic;

/// Most values one parameter may receive.
pub const MAX_VALUES: usize = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamType {
    #[default]
    Text,
    Integer,
    Number,
    Boolean,
    /// `2026-10-31`
    Date,
    /// `2026-10-01..2026-10-31`, both ends included.
    DateRange,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default, rename = "type")]
    pub kind: ParamType,
    /// Used when no value is supplied, written like a supplied value.
    #[serde(default)]
    pub default: Option<String>,
    /// Allowed values; empty means any value of the type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
    /// Accepts several values (a list).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub multiple: bool,
    /// Rendering refuses to run without a value (supplied or default).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
}

impl Parameter {
    /// A text parameter with a default, as earlier report files declare them.
    pub fn text(name: impl Into<String>, label: impl Into<String>, default: Option<String>) -> Self {
        Self { name: name.into(), label: label.into(), kind: ParamType::Text, default, choices: Vec::new(), multiple: false, required: false }
    }

    /// Reads one supplied value as this parameter's type.
    pub fn parse(&self, raw: &str) -> Result<ParamValue, String> {
        let t = raw.trim();
        let bad = |what: &str| format!("`{raw}` is not {what}");
        let date = |s: &str| NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").map_err(|_| bad("a date (YYYY-MM-DD)"));
        let value = match self.kind {
            ParamType::Text => ParamValue::Text(raw.to_owned()),
            ParamType::Integer => ParamValue::Integer(t.parse().map_err(|_| bad("a whole number"))?),
            ParamType::Number => ParamValue::Number(t.parse::<f64>().ok().filter(|n| n.is_finite()).ok_or_else(|| bad("a number"))?),
            ParamType::Boolean => ParamValue::Boolean(match t.to_ascii_lowercase().as_str() {
                "true" | "yes" | "1" => true,
                "false" | "no" | "0" => false,
                _ => return Err(bad("true or false")),
            }),
            ParamType::Date => ParamValue::Date(date(t)?),
            ParamType::DateRange => {
                let (from, to) = t.split_once("..").ok_or_else(|| bad("a date range (YYYY-MM-DD..YYYY-MM-DD)"))?;
                let (from, to) = (date(from)?, date(to)?);
                if from > to {
                    return Err(format!("`{raw}`: the range ends before it starts"));
                }
                ParamValue::DateRange(from, to)
            }
        };
        if !self.choices.is_empty() && !self.choices.iter().any(|c| self.parse_choice(c).as_ref() == Some(&value)) {
            return Err(format!("`{raw}` is not one of the allowed values ({})", self.choices.join(", ")));
        }
        Ok(value)
    }

    fn parse_choice(&self, raw: &str) -> Option<ParamValue> {
        Self { choices: Vec::new(), ..self.clone() }.parse(raw).ok()
    }

    /// Problems in the declaration itself: defaults and choices that don't fit the type.
    pub fn declaration_problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.name.trim().is_empty() || !self.name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            out.push(format!("parameter name `{}` must be letters, digits or `_`", self.name));
        }
        for c in &self.choices {
            if self.parse_choice(c).is_none() {
                out.push(format!("parameter `{}`: choice `{c}` is not a valid {:?}", self.name, self.kind));
            }
        }
        if let Some(d) = &self.default {
            for one in self.split_default(d) {
                if let Err(e) = self.parse(one) {
                    out.push(format!("parameter `{}`: default {e}", self.name));
                }
            }
        }
        out
    }

    /// A default for a multiple parameter lists its values separated by `;`.
    fn split_default<'a>(&self, d: &'a str) -> Vec<&'a str> {
        if self.multiple { d.split(';').map(str::trim).filter(|s| !s.is_empty()).collect() } else { vec![d] }
    }
}

/// A resolved parameter value.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    Text(String),
    Integer(i64),
    Number(f64),
    Boolean(bool),
    Date(NaiveDate),
    DateRange(NaiveDate, NaiveDate),
    List(Vec<ParamValue>),
}

impl ParamValue {
    /// The canonical text: ISO dates, `from..to` ranges, lists joined by `, `.
    pub fn display(&self) -> String {
        match self {
            ParamValue::Text(t) => t.clone(),
            ParamValue::Integer(i) => i.to_string(),
            ParamValue::Number(n) => n.to_string(),
            ParamValue::Boolean(b) => b.to_string(),
            ParamValue::Date(d) => d.format("%Y-%m-%d").to_string(),
            ParamValue::DateRange(a, b) => format!("{}..{}", a.format("%Y-%m-%d"), b.format("%Y-%m-%d")),
            ParamValue::List(v) => v.iter().map(ParamValue::display).collect::<Vec<_>>().join(", "),
        }
    }
}

/// Every parameter's value for one run: supplied values checked against their declaration,
/// defaults for the rest. Supplied values for undeclared names are kept as text. Missing
/// optional parameters have no entry.
pub fn resolve(declared: &[Parameter], supplied: &BTreeMap<String, Vec<String>>) -> Result<BTreeMap<String, ParamValue>, Vec<Diagnostic>> {
    let mut values = BTreeMap::new();
    let mut problems = Vec::new();
    for (i, p) in declared.iter().enumerate() {
        let at = format!("parameters[{i}]");
        let given: Vec<&str> = match supplied.get(&p.name) {
            Some(v) if !v.is_empty() => v.iter().map(String::as_str).collect(),
            _ => p.default.as_deref().map(|d| p.split_default(d)).unwrap_or_default(),
        };
        if given.is_empty() {
            if p.required {
                problems.push(Diagnostic::error(&at, format!("parameter `{}` needs a value", p.name)));
            }
            continue;
        }
        if given.len() > 1 && !p.multiple {
            problems.push(Diagnostic::error(&at, format!("parameter `{}` takes one value, got {}", p.name, given.len())));
            continue;
        }
        if given.len() > MAX_VALUES {
            problems.push(Diagnostic::error(&at, format!("parameter `{}`: more than {MAX_VALUES} values", p.name)));
            continue;
        }
        let parsed: Result<Vec<ParamValue>, String> = given.iter().map(|g| p.parse(g)).collect();
        match parsed {
            Ok(mut v) if !p.multiple => {
                if let Some(one) = v.pop() {
                    values.insert(p.name.clone(), one);
                }
            }
            Ok(v) => {
                values.insert(p.name.clone(), ParamValue::List(v));
            }
            Err(e) => problems.push(Diagnostic::error(&at, format!("parameter `{}`: {e}", p.name))),
        }
    }
    for (name, v) in supplied {
        if !declared.iter().any(|p| &p.name == name) && !v.is_empty() {
            values.insert(name.clone(), ParamValue::Text(v.join(", ")));
        }
    }
    if problems.is_empty() { Ok(values) } else { Err(problems) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(kind: ParamType) -> Parameter {
        Parameter { kind, ..Parameter::text("p", "", None) }
    }

    fn given(pairs: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), v.iter().map(|s| (*s).to_owned()).collect())).collect()
    }

    #[test]
    fn values_are_read_by_type() {
        assert_eq!(p(ParamType::Integer).parse(" 42 "), Ok(ParamValue::Integer(42)));
        assert_eq!(p(ParamType::Number).parse("2.5"), Ok(ParamValue::Number(2.5)));
        assert!(p(ParamType::Number).parse("inf").is_err());
        assert_eq!(p(ParamType::Boolean).parse("Yes"), Ok(ParamValue::Boolean(true)));
        let d = |s| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        assert_eq!(p(ParamType::Date).parse("2026-10-31"), Ok(ParamValue::Date(d("2026-10-31"))));
        assert!(p(ParamType::Date).parse("31/10/2026").is_err());
        assert_eq!(p(ParamType::DateRange).parse("2026-10-01..2026-10-31"), Ok(ParamValue::DateRange(d("2026-10-01"), d("2026-10-31"))));
        assert!(p(ParamType::DateRange).parse("2026-10-31..2026-10-01").is_err(), "reversed");
        assert_eq!(p(ParamType::DateRange).parse("2026-10-01..2026-10-31").unwrap().display(), "2026-10-01..2026-10-31");
    }

    #[test]
    fn choices_compare_parsed_values() {
        let branch = Parameter { choices: vec!["LYO".into(), "ANN".into()], ..p(ParamType::Text) };
        assert!(branch.parse("LYO").is_ok());
        assert!(branch.parse("PAR").unwrap_err().contains("LYO, ANN"));
        let n = Parameter { choices: vec!["10".into(), "20".into()], ..p(ParamType::Integer) };
        assert!(n.parse("010").is_ok(), "10 and 010 are the same number");
        let bad = Parameter { choices: vec!["x".into()], default: Some("y".into()), ..p(ParamType::Integer) };
        assert_eq!(bad.declaration_problems().len(), 2);
        assert_eq!(Parameter::text("bad name", "", None).declaration_problems().len(), 1);
    }

    #[test]
    fn resolve_applies_defaults_required_and_multiple() {
        let declared = vec![
            Parameter { required: true, ..Parameter { name: "period".into(), ..p(ParamType::DateRange) } },
            Parameter { name: "branch".into(), multiple: true, default: Some("LYO; ANN".into()), ..p(ParamType::Text) },
            Parameter { name: "min".into(), ..p(ParamType::Number) },
        ];
        let missing = resolve(&declared, &given(&[])).unwrap_err();
        assert!(missing.iter().any(|d| d.message.contains("`period` needs a value")));
        let ok = resolve(&declared, &given(&[("period", &["2026-10-01..2026-10-31"]), ("extra", &["x"])])).unwrap();
        assert_eq!(ok["branch"].display(), "LYO, ANN");
        assert!(!ok.contains_key("min"), "optional and absent");
        assert_eq!(ok["extra"], ParamValue::Text("x".into()), "undeclared values pass through as text");
        let two = resolve(&declared, &given(&[("period", &["2026-10-01..2026-10-02", "2026-11-01..2026-11-02"])])).unwrap_err();
        assert!(two[0].message.contains("takes one value"));
        let typo = resolve(&declared, &given(&[("period", &["2026-10-01..2026-10-31"]), ("min", &["ten"])])).unwrap_err();
        assert!(typo[0].message.contains("`ten` is not a number"));
    }
}
