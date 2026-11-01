//! Report parameters in queries, bound, never spliced.
//!
//! A query names parameters as `:name`; a date range as `:name.from` and `:name.to`; a list
//! parameter expands where it stands into one placeholder per value (`IN (:branches)`). The
//! scanner skips string literals, quoted identifiers, comments, PostgreSQL dollar-quoted strings
//! and `::` casts, so a `:word` inside them is left alone. Only placeholders are written into the
//! SQL; the values travel separately to the database.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use reportcraft_core::{Error, Result};
use reportcraft_model::ParamValue;

/// Most placeholders one query may receive after list expansion.
pub const MAX_BINDS: usize = 10_000;

/// A value to bind, typed so each database can bind it natively.
#[derive(Debug, Clone, PartialEq)]
pub enum BindValue {
    Null,
    Text(String),
    Integer(i64),
    Number(f64),
    Boolean(bool),
    Date(NaiveDate),
}

impl BindValue {
    /// The value as text (`None` for NULL), for databases bound through text.
    pub fn as_text(&self) -> Option<String> {
        match self {
            BindValue::Null => None,
            BindValue::Text(t) => Some(t.clone()),
            BindValue::Integer(i) => Some(i.to_string()),
            BindValue::Number(n) => Some(n.to_string()),
            BindValue::Boolean(b) => Some(b.to_string()),
            BindValue::Date(d) => Some(d.format("%Y-%m-%d").to_string()),
        }
    }
}

/// How a database writes the n-th placeholder (1-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// SQLite: `?1`, `?2`…
    Sqlite,
    /// PostgreSQL: values bound as text and cast to their type, `($1::text)::bigint`, so the
    /// server never has to guess a parameter's type.
    Postgres,
}

impl Dialect {
    fn placeholder(self, n: usize, value: &BindValue, hint: Option<&ParamValue>) -> String {
        match self {
            Dialect::Sqlite => format!("?{n}"),
            Dialect::Postgres => {
                let cast = match (value, hint) {
                    (BindValue::Integer(_), _) | (BindValue::Null, Some(ParamValue::Integer(_))) => "bigint",
                    (BindValue::Number(_), _) | (BindValue::Null, Some(ParamValue::Number(_))) => "numeric",
                    (BindValue::Boolean(_), _) | (BindValue::Null, Some(ParamValue::Boolean(_))) => "boolean",
                    (BindValue::Date(_), _) => "date",
                    _ => "text",
                };
                if cast == "text" { format!("(${n}::text)") } else { format!("((${n}::text)::{cast})") }
            }
        }
    }
}

/// The SQL with placeholders and the values to bind, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Bound {
    pub sql: String,
    pub values: Vec<BindValue>,
}

fn scalar(v: &ParamValue) -> Result<BindValue> {
    Ok(match v {
        ParamValue::Text(t) => BindValue::Text(t.clone()),
        ParamValue::Integer(i) => BindValue::Integer(*i),
        ParamValue::Number(n) => BindValue::Number(*n),
        ParamValue::Boolean(b) => BindValue::Boolean(*b),
        ParamValue::Date(d) => BindValue::Date(*d),
        ParamValue::DateRange(..) => return Err(Error::Invalid("a date range is used as `:name.from` and `:name.to`".into())),
        ParamValue::List(_) => return Err(Error::Invalid("a list of lists cannot be bound".into())),
    })
}

/// Replaces every `:name` in `sql` with placeholders for `values`. A name the report doesn't
/// declare is an error (so a typo never silently becomes NULL); `declared` names without a value
/// bind NULL.
pub fn bind(sql: &str, values: &BTreeMap<String, ParamValue>, declared: &[&str], dialect: Dialect) -> Result<Bound> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len());
    let mut binds: Vec<BindValue> = Vec::new();
    let mut i = 0;
    let at = |i: usize| chars.get(i).copied();
    while let Some(c) = at(i) {
        // Skip the parts of SQL where `:word` is not a parameter, copying them as they are.
        let skip_to = match c {
            '\'' => Some(end_of_quoted(&chars, i, '\'')),
            '"' => Some(end_of_quoted(&chars, i, '"')),
            '-' if at(i + 1) == Some('-') => Some(chars.iter().skip(i).position(|&c| c == '\n').map_or(chars.len(), |n| i + n)),
            '/' if at(i + 1) == Some('*') => Some(end_of_block_comment(&chars, i)),
            '$' => end_of_dollar_quote(&chars, i),
            ':' if at(i + 1) == Some(':') => Some(i + 2),
            _ => None,
        };
        if let Some(end) = skip_to {
            out.extend(chars.get(i..end).unwrap_or_default());
            i = end.max(i + 1);
            continue;
        }
        let starts_name = |c: Option<char>| c.is_some_and(|c| c.is_alphabetic() || c == '_');
        if c != ':' || !starts_name(at(i + 1)) || i.checked_sub(1).and_then(at) == Some(':') {
            out.push(c);
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while at(j).is_some_and(|c| c.is_alphanumeric() || c == '_') {
            j += 1;
        }
        let name: String = chars.get(i + 1..j).unwrap_or_default().iter().collect();
        let part = ["from", "to"].into_iter().find(|p| {
            at(j) == Some('.')
                && chars.get(j + 1..j + 1 + p.len()).is_some_and(|s| s.iter().copied().eq(p.chars()))
                && !at(j + 1 + p.len()).is_some_and(|c| c.is_alphanumeric() || c == '_')
        });
        if !declared.contains(&name.as_str()) {
            return Err(Error::Invalid(format!("the query uses `:{name}`, which the report doesn't declare as a parameter")));
        }
        let value = values.get(&name);
        let (pieces, consumed): (Vec<BindValue>, usize) = match (value, part) {
            (Some(ParamValue::DateRange(from, to)), Some(p)) => (vec![BindValue::Date(if p == "from" { *from } else { *to })], 1 + p.len()),
            (Some(ParamValue::DateRange(..)), None) => {
                return Err(Error::Invalid(format!("`:{name}` is a date range: use `:{name}.from` and `:{name}.to`")));
            }
            (Some(_), Some(p)) => return Err(Error::Invalid(format!("`:{name}.{p}`: only date ranges have `.from` and `.to`"))),
            (None, Some(p)) => (vec![BindValue::Null], 1 + p.len()),
            (Some(ParamValue::List(items)), None) => (items.iter().map(scalar).collect::<Result<_>>()?, 0),
            (Some(v), None) => (vec![scalar(v)?], 0),
            (None, None) => (vec![BindValue::Null], 0),
        };
        if binds.len() + pieces.len() > MAX_BINDS {
            return Err(Error::TooLarge(format!("more than {MAX_BINDS} bound values")));
        }
        let hint = match value {
            Some(ParamValue::List(items)) => items.first(),
            other => other,
        };
        let placeholders: Vec<String> = pieces
            .into_iter()
            .map(|v| {
                binds.push(v);
                let n = binds.len();
                binds.last().map(|v| dialect.placeholder(n, v, hint)).unwrap_or_default()
            })
            .collect();
        out.push_str(&placeholders.join(", "));
        i = j + consumed;
    }
    Ok(Bound { sql: out, values: binds })
}

/// One past a quoted literal or identifier; a doubled quote is part of it.
fn end_of_quoted(chars: &[char], start: usize, q: char) -> usize {
    let backslashes = q == '\'' && start.checked_sub(1).and_then(|p| chars.get(p)).is_some_and(|c| c.eq_ignore_ascii_case(&'e'));
    let mut i = start + 1;
    while let Some(&c) = chars.get(i) {
        if backslashes && c == '\\' {
            i += 2;
            continue;
        }
        if c == q {
            if chars.get(i + 1) == Some(&q) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    chars.len()
}

/// One past a `/* … */` comment; PostgreSQL nests them.
fn end_of_block_comment(chars: &[char], start: usize) -> usize {
    let (mut depth, mut i) = (0usize, start);
    while i < chars.len() {
        match (chars.get(i), chars.get(i + 1)) {
            (Some('/'), Some('*')) => {
                depth += 1;
                i += 2;
            }
            (Some('*'), Some('/')) => {
                depth = depth.saturating_sub(1);
                i += 2;
                if depth == 0 {
                    return i;
                }
            }
            _ => i += 1,
        }
    }
    chars.len()
}

/// One past a `$tag$ … $tag$` string, if `start` opens one (`$1` is not a tag).
fn end_of_dollar_quote(chars: &[char], start: usize) -> Option<usize> {
    let mut j = start + 1;
    while chars.get(j).is_some_and(|c| c.is_alphanumeric() || *c == '_') {
        j += 1;
    }
    if chars.get(j) != Some(&'$') || chars.get(start + 1).is_some_and(char::is_ascii_digit) {
        return None;
    }
    let tag: Vec<char> = chars.get(start..=j)?.to_vec();
    let body = j + 1;
    (body..chars.len()).find(|&k| chars.get(k..k + tag.len()) == Some(tag.as_slice())).map(|k| k + tag.len()).or(Some(chars.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn values() -> BTreeMap<String, ParamValue> {
        BTreeMap::from([
            ("branch".to_owned(), ParamValue::Text("LYO".into())),
            ("min".to_owned(), ParamValue::Integer(3)),
            ("period".to_owned(), ParamValue::DateRange(d("2026-10-01"), d("2026-10-31"))),
            ("branches".to_owned(), ParamValue::List(vec![ParamValue::Text("LYO".into()), ParamValue::Text("ANN".into())])),
        ])
    }

    const DECLARED: &[&str] = &["branch", "min", "period", "branches", "unset"];

    #[test]
    fn binds_names_ranges_and_lists() {
        let b = bind(
            "SELECT * FROM r WHERE branch = :branch AND days >= :min AND started BETWEEN :period.from AND :period.to AND b IN (:branches) AND x = :unset",
            &values(),
            DECLARED,
            Dialect::Sqlite,
        )
        .unwrap();
        assert_eq!(b.sql, "SELECT * FROM r WHERE branch = ?1 AND days >= ?2 AND started BETWEEN ?3 AND ?4 AND b IN (?5, ?6) AND x = ?7");
        assert_eq!(
            b.values,
            vec![
                BindValue::Text("LYO".into()),
                BindValue::Integer(3),
                BindValue::Date(d("2026-10-01")),
                BindValue::Date(d("2026-10-31")),
                BindValue::Text("LYO".into()),
                BindValue::Text("ANN".into()),
                BindValue::Null,
            ]
        );
    }

    #[test]
    fn postgres_placeholders_cast_text_to_the_type() {
        let b = bind("SELECT :min, :branch, :period.to", &values(), DECLARED, Dialect::Postgres).unwrap();
        assert_eq!(b.sql, "SELECT (($1::text)::bigint), ($2::text), (($3::text)::date)");
        assert_eq!(b.values[0].as_text().as_deref(), Some("3"));
    }

    #[test]
    fn leaves_strings_comments_casts_and_dollar_quotes_alone() {
        let sql = r#"SELECT ':branch', "a:branch", x::text, E'it\'s :branch', $$ :branch $$, $q$ :branch $q$ -- :branch
/* :branch /* nested :branch */ :branch */ FROM t WHERE y = :branch"#;
        let b = bind(sql, &values(), DECLARED, Dialect::Sqlite).unwrap();
        assert_eq!(b.values, vec![BindValue::Text("LYO".into())], "only the last one is a parameter");
        assert!(b.sql.ends_with("WHERE y = ?1"), "{}", b.sql);
        assert!(b.sql.starts_with(r#"SELECT ':branch', "a:branch", x::text, E'it\'s :branch', $$ :branch $$, $q$ :branch $q$"#));
    }

    #[test]
    fn mistakes_are_errors() {
        let e = |sql: &str| bind(sql, &values(), DECLARED, Dialect::Sqlite).unwrap_err().to_string();
        assert!(e("SELECT :brnch").contains("doesn't declare"));
        assert!(e("SELECT :period").contains(":period.from"));
        assert!(e("SELECT :min.from").contains("only date ranges"));
        assert_eq!(bind("SELECT 1 WHERE a = 'x''y' AND b = :min", &values(), DECLARED, Dialect::Sqlite).unwrap().values.len(), 1);
        assert_eq!(bind("SELECT $1, ?", &values(), DECLARED, Dialect::Sqlite).unwrap().sql, "SELECT $1, ?", "foreign placeholders untouched");
    }

    #[test]
    fn a_value_never_reaches_the_sql_text() {
        let evil = BTreeMap::from([("branch".to_owned(), ParamValue::Text("x'; DROP TABLE r; --".into()))]);
        let b = bind("SELECT * FROM r WHERE branch = :branch", &evil, DECLARED, Dialect::Postgres).unwrap();
        assert!(!b.sql.contains("DROP"));
        assert_eq!(b.values, vec![BindValue::Text("x'; DROP TABLE r; --".into())]);
    }
}
