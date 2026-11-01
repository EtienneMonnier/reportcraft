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
use reportcraft_data::{ColumnInfo, DataSource, ForeignKey, Schema, SchemaSource, Table, TableKind};
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

    /// A source for schema discovery and previews only (no report query).
    pub fn database(path: impl Into<PathBuf>) -> Self {
        Self::new(path, "")
    }

    /// Opens the file read-only, without ATTACH, with the time limit armed from now.
    fn open(&self) -> Result<Connection> {
        if !self.path.is_file() {
            return Err(Error::NotFound(format!("SQLite database {}", self.path.display())));
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI;
        let mut conn = Connection::open_with_flags(Self::file_uri(&self.path), flags).map_err(|e| io(&self.path, &e))?;
        // No ATTACH: the query reads this file and nothing else.
        conn.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0).map_err(|e| io(&self.path, &e))?;
        conn.pragma_update(None, "query_only", true).map_err(|e| io(&self.path, &e))?;
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
        run(&self.open()?, &self.query, max_rows)
    }
}

/// Runs one read-only statement and collects its rows.
fn run(conn: &Connection, sql: &str, max_rows: usize) -> Result<RowSet> {
    let mut stmt = conn.prepare(sql).map_err(|e| query_error(&e))?;
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

/// Most tables a schema listing returns.
const MAX_TABLES: usize = 10_000;

impl SchemaSource for SqliteSource {
    fn schema(&self) -> Result<Schema> {
        let conn = self.open()?;
        let mut stmt = conn
            .prepare("SELECT name, type FROM sqlite_schema WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite!_%' ESCAPE '!' ORDER BY name")
            .map_err(|e| query_error(&e))?;
        let listed = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| query_error(&e))?
            .take(MAX_TABLES.saturating_add(1))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| query_error(&e))?;
        if listed.len() > MAX_TABLES {
            return Err(Error::TooLarge(format!("more than {MAX_TABLES} tables and views")));
        }
        let mut tables = Vec::with_capacity(listed.len());
        for (name, kind) in listed {
            let kind = if kind == "view" { TableKind::View } else { TableKind::Table };
            tables.push(Table { columns: columns(&conn, &name)?, foreign_keys: foreign_keys(&conn, &name)?, name, kind });
        }
        // A foreign key may name no target columns: it then points at the target's primary key.
        let primary_keys: Vec<(String, Vec<String>)> = tables.iter().map(|t| (t.name.clone(), primary_key(t))).collect();
        for fk in tables.iter_mut().flat_map(|t| t.foreign_keys.iter_mut()) {
            if fk.references_columns.iter().all(String::is_empty)
                && let Some((_, pk)) = primary_keys.iter().find(|(n, _)| n.eq_ignore_ascii_case(&fk.references_table))
            {
                fk.references_columns.clone_from(pk);
            }
        }
        Ok(Schema { tables })
    }

    fn preview(&self, table: &str, limit: usize) -> Result<RowSet> {
        let schema = self.schema()?;
        let found = schema.table(table).ok_or_else(|| Error::NotFound(format!("table or view `{table}`")))?;
        let limit = limit.min(limits::MAX_ROWS);
        run(&self.open()?, &format!("SELECT * FROM {} LIMIT {limit}", quote_identifier(&found.name)), limit)
    }
}

/// `"name"` with inner quotes doubled: safe for any table name.
fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn primary_key(table: &Table) -> Vec<String> {
    let mut pk: Vec<(u32, String)> = table.columns.iter().filter_map(|c| c.primary_key.map(|p| (p, c.name.clone()))).collect();
    pk.sort();
    pk.into_iter().map(|(_, n)| n).collect()
}

fn columns(conn: &Connection, table: &str) -> Result<Vec<ColumnInfo>> {
    let mut stmt = conn.prepare(r#"SELECT name, type, "notnull", pk FROM pragma_table_info(?1)"#).map_err(|e| query_error(&e))?;
    let rows = stmt
        .query_map([table], |r| {
            let declared: String = r.get::<_, Option<String>>(1)?.unwrap_or_default();
            let pk: i64 = r.get(3)?;
            Ok(ColumnInfo {
                name: r.get(0)?,
                field_type: declared_type(&declared).unwrap_or(FieldType::Text),
                declared_type: declared,
                nullable: r.get::<_, i64>(2)? == 0,
                primary_key: u32::try_from(pk).ok().filter(|p| *p > 0),
            })
        })
        .map_err(|e| query_error(&e))?
        .take(limits::MAX_COLUMNS.saturating_add(1))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| query_error(&e))?;
    if rows.len() > limits::MAX_COLUMNS {
        return Err(Error::TooLarge(format!("`{table}` has more than {} columns", limits::MAX_COLUMNS)));
    }
    // SQLite lets NULL into other primary-key columns unless they say NOT NULL; only a single
    // INTEGER PRIMARY KEY (the rowid) can never be NULL.
    let mut rows = rows;
    let pk_count = rows.iter().filter(|c| c.primary_key.is_some()).count();
    if pk_count == 1
        && let Some(rowid) = rows.iter_mut().find(|c| c.primary_key.is_some() && c.declared_type.eq_ignore_ascii_case("INTEGER"))
    {
        rowid.nullable = false;
    }
    Ok(rows)
}

fn foreign_keys(conn: &Connection, table: &str) -> Result<Vec<ForeignKey>> {
    let mut stmt =
        conn.prepare(r#"SELECT id, "table", "from", "to" FROM pragma_foreign_key_list(?1) ORDER BY id, seq"#).map_err(|e| query_error(&e))?;
    let rows = stmt
        .query_map([table], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?)))
        .map_err(|e| query_error(&e))?
        .take(limits::MAX_COLUMNS)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| query_error(&e))?;
    let mut keys: Vec<(i64, ForeignKey)> = Vec::new();
    for (id, target, from, to) in rows {
        match keys.last_mut() {
            Some((last, fk)) if *last == id => {
                fk.columns.push(from);
                fk.references_columns.push(to.unwrap_or_default());
            }
            _ => keys.push((id, ForeignKey { columns: vec![from], references_table: target, references_columns: vec![to.unwrap_or_default()] })),
        }
    }
    Ok(keys.into_iter().map(|(_, fk)| fk).collect())
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
    fn lists_tables_views_columns_and_keys() {
        let path = sample_db("schema");
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                r#"CREATE TABLE branch (code TEXT NOT NULL, region TEXT, PRIMARY KEY (code));
                   CREATE TABLE "odd ""name""" (x, PRIMARY KEY (x));
                   CREATE TABLE booking (id INTEGER PRIMARY KEY, rental_id INT REFERENCES rental(id), branch_code TEXT REFERENCES branch);
                   CREATE VIEW lyon AS SELECT * FROM rental WHERE branch = 'Lyon';"#,
            )
            .unwrap();
        let schema = SqliteSource::database(&path).schema().unwrap();
        let names: Vec<&str> = schema.tables.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["booking", "branch", "lyon", "odd \"name\"", "rental"]);
        let rental = schema.table("RENTAL").unwrap();
        assert_eq!(rental.kind, TableKind::Table);
        let id = &rental.columns[0];
        assert_eq!((id.name.as_str(), id.declared_type.as_str(), id.primary_key, id.nullable), ("id", "INTEGER", Some(1), false));
        assert_eq!(rental.columns[3].field_type, FieldType::Integer);
        assert!(rental.columns[1].nullable);
        assert!(!schema.table("branch").unwrap().columns[0].nullable, "declared NOT NULL");
        let odd = schema.table("odd \"name\"").unwrap();
        assert!(odd.columns[0].nullable && odd.columns[0].declared_type.is_empty());
        assert_eq!(schema.table("lyon").unwrap().kind, TableKind::View);
        let fks = &schema.table("booking").unwrap().foreign_keys;
        assert_eq!(fks.len(), 2);
        let to_rental = fks.iter().find(|f| f.references_table == "rental").unwrap();
        assert_eq!((to_rental.columns.clone(), to_rental.references_columns.clone()), (vec!["rental_id".to_owned()], vec!["id".to_owned()]));
        let to_branch = fks.iter().find(|f| f.references_table == "branch").unwrap();
        assert_eq!(to_branch.references_columns, ["code"], "an implicit target is the primary key");
    }

    #[test]
    fn previews_any_table_name_safely() {
        let path = sample_db("preview");
        Connection::open(&path)
            .unwrap()
            .execute_batch(r#"CREATE TABLE "x""; DROP TABLE rental; --" (a); INSERT INTO "x""; DROP TABLE rental; --" VALUES (1), (2), (3);"#)
            .unwrap();
        let src = SqliteSource::database(&path);
        let rs = src.preview(r#"x"; DROP TABLE rental; --"#, 2).unwrap();
        assert_eq!(rs.rows.len(), 2);
        assert_eq!(src.preview("rental", 10).unwrap().rows.len(), 3, "rental is still there");
        assert!(matches!(src.preview("nope", 5), Err(Error::NotFound(_))));
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
