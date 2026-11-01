//! ReportCraft data (L2): data sources behind one trait.
//!
//! A new kind of source (PostgreSQL, SQLite, ODBC…) is a new `DataSource` implementation; nothing
//! else matches on source kinds. Sources are read-only: ReportCraft never writes to them.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod schema;

pub use schema::{ColumnInfo, ForeignKey, Schema, SchemaSource, Table, TableKind};

use reportcraft_core::{Column, Error, FieldType, Result, RowSet, Value, limits};

/// Something that hands rows to a report.
pub trait DataSource {
    /// A short description for listings ("CSV, 3 columns").
    fn describe(&self) -> String;
    /// Reads at most `limit` rows (and never more than `limits::MAX_ROWS`).
    fn read(&self, limit: Option<usize>) -> Result<RowSet>;
}

/// Rows already in memory.
#[derive(Debug, Clone, Default)]
pub struct MemorySource {
    pub rows: RowSet,
}

impl DataSource for MemorySource {
    fn describe(&self) -> String {
        format!("memory, {} columns, {} rows", self.rows.columns.len(), self.rows.rows.len())
    }

    fn read(&self, limit: Option<usize>) -> Result<RowSet> {
        let n = limit.unwrap_or(limits::MAX_ROWS).min(limits::MAX_ROWS);
        Ok(RowSet { columns: self.rows.columns.clone(), rows: self.rows.rows.iter().take(n).cloned().collect() })
    }
}

/// CSV text with a header row. Column types are inferred from the values unless given.
#[derive(Debug, Clone)]
pub struct CsvSource {
    text: String,
    separator: char,
    types: Vec<(String, FieldType)>,
}

impl CsvSource {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let separator = detect_separator(&text);
        Self { text, separator, types: Vec::new() }
    }

    /// Declares a column's type instead of inferring it.
    pub fn with_type(mut self, column: &str, field_type: FieldType) -> Self {
        self.types.push((column.to_owned(), field_type));
        self
    }
}

impl DataSource for CsvSource {
    fn describe(&self) -> String {
        format!("CSV, separator {:?}", self.separator)
    }

    fn read(&self, limit: Option<usize>) -> Result<RowSet> {
        let max = limit.unwrap_or(limits::MAX_ROWS).min(limits::MAX_ROWS);
        let mut records = parse_csv(&self.text, self.separator, max.saturating_add(1))?.into_iter();
        let header = records.next().ok_or_else(|| Error::Invalid("CSV has no header row".into()))?;
        if header.len() > limits::MAX_COLUMNS {
            return Err(Error::TooLarge(format!("more than {} columns", limits::MAX_COLUMNS)));
        }
        let raw: Vec<Vec<String>> = records.collect();
        let columns: Vec<Column> = header
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let declared = self.types.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, t)| *t);
                let field_type = declared.unwrap_or_else(|| infer_type(raw.iter().filter_map(|r| r.get(i).map(String::as_str))));
                Column { name: name.trim().to_owned(), field_type }
            })
            .collect();
        let rows =
            raw.iter().map(|r| columns.iter().enumerate().map(|(i, c)| r.get(i).map_or(Value::Null, |s| c.field_type.parse(s))).collect()).collect();
        Ok(RowSet { columns, rows })
    }
}

fn detect_separator(text: &str) -> char {
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    [',', ';', '\t', '|'].into_iter().max_by_key(|c| first.matches(*c).count()).unwrap_or(',')
}

fn infer_type<'a>(values: impl Iterator<Item = &'a str>) -> FieldType {
    let (mut int, mut num, mut any) = (true, true, false);
    for v in values.map(str::trim).filter(|v| !v.is_empty()).take(1_000) {
        any = true;
        int &= v.parse::<i64>().is_ok();
        num &= v.parse::<f64>().is_ok_and(f64::is_finite);
    }
    match (any, int, num) {
        (true, true, _) => FieldType::Integer,
        (true, false, true) => FieldType::Number,
        _ => FieldType::Text,
    }
}

/// RFC 4180-style parser: quoted fields, doubled quotes, CRLF or LF. Stops after `max_records`.
fn parse_csv(text: &str, sep: char, max_records: usize) -> Result<Vec<Vec<String>>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            '\r' => {}
            '\n' => {
                record.push(std::mem::take(&mut field));
                if record.len() > limits::MAX_COLUMNS {
                    return Err(Error::TooLarge(format!("more than {} columns", limits::MAX_COLUMNS)));
                }
                if !(record.len() == 1 && record.first().is_some_and(String::is_empty)) {
                    records.push(std::mem::take(&mut record));
                    if records.len() >= max_records {
                        return Ok(records);
                    }
                }
                record.clear();
            }
            c if c == sep => record.push(std::mem::take(&mut field)),
            _ => field.push(c),
        }
    }
    if quoted {
        return Err(Error::Invalid("CSV ends inside a quoted field".into()));
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_quoted_fields_and_infers_types() {
        let src = CsvSource::new("name;qty;price\n\"Tent; large\";2;10.5\r\n\"Say \"\"hi\"\"\";3;4\n");
        let rs = src.read(None).unwrap();
        assert_eq!(rs.columns.iter().map(|c| c.field_type).collect::<Vec<_>>(), [FieldType::Text, FieldType::Integer, FieldType::Number]);
        assert_eq!(rs.rows[0][0], Value::Text("Tent; large".into()));
        assert_eq!(rs.rows[1][0], Value::Text("Say \"hi\"".into()));
        assert_eq!(rs.rows[1][2], Value::Number(4.0));
    }

    #[test]
    fn limit_caps_rows_and_unterminated_quote_is_an_error() {
        let rs = CsvSource::new("a\n1\n2\n3\n").read(Some(2)).unwrap();
        assert_eq!(rs.rows.len(), 2);
        assert!(CsvSource::new("a\n\"1\n").read(None).is_err());
        assert!(CsvSource::new("").read(None).is_err());
    }
}
