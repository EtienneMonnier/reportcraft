//! ReportCraft SQLite source (L3, native only): reads rows from a SQLite database file with one
//! query, read-only.
//!
//! Read-only is enforced three ways: the file is opened read-only, the query must be a single
//! statement SQLite reports as read-only, and `ATTACH` is disabled so the query cannot reach
//! another file. Queries are stopped after a time limit and rows are capped.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use reportcraft_core::{Column, Error, FieldType, Result, RowSet, Value, limits};
use reportcraft_data::DataSource;
use rusqlite::limits::Limit;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};

/// How long a query may run by default.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest query text accepted.
const MAX_QUERY_LEN: usize = 1024 * 1024;
/// How many SQLite virtual-machine steps between two time checks.
const PROGRESS_STEPS: i32 = 10_000;

/// A query over one SQLite database file.
#[derive(Debug, Clone)]
pub struct SqliteSource {
    path: PathBuf,
    query: String,
    timeout: Duration,
}

impl SqliteSource {
    pub fn new(path: impl Into<PathBuf>, query: impl Into<String>) -> Self {
        Self { path: path.into(), query: query.into(), timeout: DEFAULT_TIMEOUT }
    }

    /// Stops the query once it has run this long.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn open(&self) -> Result<Connection> {
        if !self.path.is_file() {
            return Err(Error::NotFound(format!("SQLite database {}", self.path.display())));
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI;
        let conn = Connection::open_with_flags(Self::file_uri(&self.path), flags).map_err(|e| io(&self.path, &e))?;
        // No ATTACH: the query reads this file and nothing else.
        conn.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0).map_err(|e| io(&self.path, &e))?;
        conn.pragma_update(None, "query_only", true).map_err(|e| io(&self.path, &e))?;
        Ok(conn)
    }

    /// A `file:` URI that keeps the path literal (no query parameters smuggled in the name).
    fn file_uri(path: &Path) -> String {
        let mut uri = String::from("file:");
        for c in path.to_string_lossy().chars() {
            match c {
                '?' => uri.push_str("%3f"),
                '#' => uri.push_str("%23"),
                '%' => uri.push_str("%25"),
                _ => uri.push(c),
            }
        }
        uri.push_str("?mode=ro");
        uri
    }
}

fn io(path: &Path, e: &rusqlite::Error) -> Error {
    Error::Io(format!("{}: {e}", path.display()))
}

fn query_error(e: &rusqlite::Error) -> Error {
    match e {
        rusqlite::Error::MultipleStatement => Error::Invalid("the query must be a single statement".into()),
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::OperationInterrupted => {
            Error::TooLarge("the query ran past its time limit and was stopped".into())
        }
        other => Error::Invalid(format!("query: {other}")),
    }
}

impl DataSource for SqliteSource {
    fn describe(&self) -> String {
        format!("SQLite {}", self.path.display())
    }

    fn read(&self, limit: Option<usize>) -> Result<RowSet> {
        if self.query.len() > MAX_QUERY_LEN {
            return Err(Error::TooLarge("query text longer than 1 MiB".into()));
        }
        let max_rows = limit.unwrap_or(limits::MAX_ROWS).min(limits::MAX_ROWS);
        let mut conn = self.open()?;
        // The clock is read off wasm only: this crate is native-only.
        #[allow(clippy::disallowed_methods)]
        let deadline = Instant::now().checked_add(self.timeout);
        conn.progress_handler(
            PROGRESS_STEPS,
            Some(move || {
                #[allow(clippy::disallowed_methods)]
                let now = Instant::now();
                deadline.is_some_and(|d| now >= d)
            }),
        )
        .map_err(|e| io(&self.path, &e))?;

        let mut stmt = conn.prepare(&self.query).map_err(|e| query_error(&e))?;
        if !stmt.readonly() {
            return Err(Error::Invalid("only queries that read data are allowed (SELECT, WITH …)".into()));
        }
        let names: Vec<String> = stmt.column_names().map(str::to_owned).collect();
        if names.is_empty() {
            return Err(Error::Invalid("the query returns no columns".into()));
        }
        if names.len() > limits::MAX_COLUMNS {
            return Err(Error::TooLarge(format!("more than {} columns", limits::MAX_COLUMNS)));
        }
        let declared: Vec<Option<FieldType>> = stmt.columns().map(|c| c.decl_type().and_then(declared_type)).collect();

        let mut rows = Vec::new();
        let mut cursor = stmt.query([]).map_err(|e| query_error(&e))?;
        while rows.len() < max_rows {
            let Some(row) = cursor.next().map_err(|e| query_error(&e))? else { break };
            let mut values = Vec::with_capacity(names.len());
            for i in 0..names.len() {
                values.push(row.get_ref(i).map(to_value).map_err(|e| query_error(&e))?);
            }
            rows.push(values);
        }
        let columns = names
            .into_iter()
            .enumerate()
            .map(|(i, name)| {
                let field_type = declared.get(i).copied().flatten().unwrap_or_else(|| observed_type(&rows, i));
                Column { name, field_type }
            })
            .collect();
        Ok(RowSet { columns, rows })
    }
}

/// SQLite's column affinity rules, read from the declared type.
fn declared_type(decl: &str) -> Option<FieldType> {
    let d = decl.to_ascii_uppercase();
    if d.contains("BOOL") {
        Some(FieldType::Bool)
    } else if d.contains("INT") {
        Some(FieldType::Integer)
    } else if ["REAL", "FLOA", "DOUB", "NUM", "DEC"].iter().any(|k| d.contains(k)) {
        Some(FieldType::Number)
    } else if ["CHAR", "CLOB", "TEXT", "DATE", "TIME"].iter().any(|k| d.contains(k)) {
        Some(FieldType::Text)
    } else {
        None
    }
}

/// The type of the values actually returned, for expressions without a declared type.
fn observed_type(rows: &[Vec<Value>], column: usize) -> FieldType {
    let mut seen = rows.iter().filter_map(|r| r.get(column)).filter(|v| !v.is_null());
    let Some(first) = seen.next() else { return FieldType::Text };
    let mut t = match first {
        Value::Int(_) => FieldType::Integer,
        Value::Number(_) => FieldType::Number,
        Value::Bool(_) => FieldType::Bool,
        Value::Text(_) | Value::Null => FieldType::Text,
    };
    for v in seen.take(1_000) {
        t = match (t, v) {
            (FieldType::Integer, Value::Int(_)) => FieldType::Integer,
            (FieldType::Integer | FieldType::Number, Value::Int(_) | Value::Number(_)) => FieldType::Number,
            _ => return FieldType::Text,
        };
    }
    t
}

fn to_value(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Int(i),
        ValueRef::Real(f) if f.is_finite() => Value::Number(f),
        ValueRef::Real(_) => Value::Null,
        ValueRef::Text(t) => Value::Text(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::Text(format!("<binary, {} bytes>", b.len())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A database file with a few rentals, in a fresh temporary folder.
    fn sample_db(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reportcraft-sqlite-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rentals.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE rental (id INTEGER PRIMARY KEY, branch TEXT, item VARCHAR(40), days INT, amount NUMERIC, paid BOOLEAN, scan BLOB);
             INSERT INTO rental (branch, item, days, amount, paid, scan) VALUES
               ('Lyon', 'Scaffold tower', 12, 540.0, 1, x'0102'),
               ('Annecy', 'Scissor lift', 4, 720.5, 0, NULL),
               ('Lyon', NULL, 3, NULL, NULL, NULL);",
        )
        .unwrap();
        path
    }

    #[test]
    fn reads_typed_columns_and_values() {
        let path = sample_db("typed");
        let rs =
            SqliteSource::new(&path, "SELECT branch, item, days, amount, paid, scan, days * 2 AS double FROM rental ORDER BY id").read(None).unwrap();
        let types: Vec<FieldType> = rs.columns.iter().map(|c| c.field_type).collect();
        use FieldType::{Bool, Integer, Number, Text};
        assert_eq!(types, [Text, Text, Integer, Number, Bool, Text, Integer]);
        assert_eq!(rs.rows.len(), 3);
        assert_eq!(rs.rows[0][0], Value::Text("Lyon".into()));
        assert_eq!(rs.rows[1][3], Value::Number(720.5));
        assert_eq!(rs.rows[0][5], Value::Text("<binary, 2 bytes>".into()));
        assert!(rs.rows[2][1].is_null());
        assert_eq!(rs.rows[2][6], Value::Int(6));
    }

    #[test]
    fn limit_caps_rows() {
        let path = sample_db("limit");
        let rs = SqliteSource::new(&path, "SELECT * FROM rental").read(Some(2)).unwrap();
        assert_eq!(rs.rows.len(), 2);
    }

    #[test]
    fn refuses_anything_that_is_not_a_single_read() {
        let path = sample_db("readonly");
        for q in [
            "DELETE FROM rental",
            "INSERT INTO rental (branch) VALUES ('x')",
            "DROP TABLE rental",
            "PRAGMA query_only = OFF",
            "SELECT 1; DELETE FROM rental",
            "ATTACH DATABASE 'other.db' AS other",
            "CREATE TABLE t (a)",
        ] {
            assert!(SqliteSource::new(&path, q).read(None).is_err(), "{q} should be refused");
        }
        let rs = SqliteSource::new(&path, "SELECT count(*) AS n FROM rental").read(None).unwrap();
        assert_eq!(rs.rows[0][0], Value::Int(3), "nothing was written");
    }

    #[test]
    fn missing_file_bad_sql_and_runaway_queries_are_errors() {
        assert!(matches!(SqliteSource::new("/nonexistent/x.db", "SELECT 1").read(None), Err(Error::NotFound(_))));
        let path = sample_db("errors");
        assert!(matches!(SqliteSource::new(&path, "SELEC nope").read(None), Err(Error::Invalid(_))));
        assert!(SqliteSource::new(&path, "SELECT * FROM missing_table").read(None).is_err());
        let endless = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) SELECT count(*) FROM n";
        let r = SqliteSource::new(&path, endless).with_timeout(Duration::from_millis(200)).read(None);
        assert!(matches!(r, Err(Error::TooLarge(_))), "{r:?}");
    }

    #[test]
    fn a_file_name_cannot_inject_uri_parameters() {
        let path = sample_db("uri");
        let tricky = path.with_file_name("rentals.db?mode=rwc");
        std::fs::copy(&path, &tricky).unwrap();
        let rs = SqliteSource::new(&tricky, "SELECT count(*) FROM rental").read(None).unwrap();
        assert_eq!(rs.rows[0][0], Value::Int(3));
        assert!(SqliteSource::new(&tricky, "DELETE FROM rental").read(None).is_err());
    }
}
