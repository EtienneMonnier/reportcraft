//! ReportCraft PostgreSQL source (L3, native only): reads rows with one query and describes the
//! database's schema, read-only.
//!
//! Every query runs inside a `READ ONLY` transaction that is rolled back at the end, with a
//! statement timeout. The query must prepare as a single statement and is read through a cursor
//! (`DECLARE … CURSOR FOR`), which only accepts `SELECT` and `VALUES`, in batches up to the row
//! cap, so a large result is never loaded whole. Values come back as text and are converted by
//! column type. The connection string is never written to errors or logs: it may hold a password.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::str::FromStr;
use std::time::Duration;

use postgres::error::SqlState;
use postgres::types::{ToSql, Type};
use postgres::{Client, Config, NoTls, SimpleQueryMessage, Transaction};
use reportcraft_core::{Column, Error, FieldType, Result, RowSet, Value, limits};
use reportcraft_data::{BindValue, ColumnInfo, DataSource, ForeignKey, Schema, SchemaSource, Table, TableKind};

/// How long a query may run by default.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_QUERY_LEN: usize = 1024 * 1024;
const MAX_TABLES: usize = 10_000;
/// Rows fetched from the cursor per round trip.
const BATCH: usize = 1_000;
const CURSOR: &str = "reportcraft_rows";

/// A query over one PostgreSQL database.
#[derive(Clone)]
pub struct PostgresSource {
    /// libpq-style connection string or `postgresql://` URL. Never printed.
    conninfo: String,
    query: String,
    binds: Vec<BindValue>,
    timeout: Duration,
}

impl std::fmt::Debug for PostgresSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresSource")
            .field("query", &self.query)
            .field("binds", &self.binds.len())
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl PostgresSource {
    pub fn new(conninfo: impl Into<String>, query: impl Into<String>) -> Self {
        Self { conninfo: conninfo.into(), query: query.into(), binds: Vec::new(), timeout: DEFAULT_TIMEOUT }
    }

    /// Values for the query's `$1`, `$2`… placeholders, sent as text (see `reportcraft_data::bind`,
    /// whose PostgreSQL placeholders cast them to their type). Values are never printed.
    pub fn with_binds(mut self, binds: Vec<BindValue>) -> Self {
        self.binds = binds;
        self
    }

    /// A source for schema discovery and previews only (no report query).
    pub fn database(conninfo: impl Into<String>) -> Self {
        Self::new(conninfo, "")
    }

    /// Cancels the query once it has run this long.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn connect(&self) -> Result<Client> {
        let mut config = Config::from_str(&self.conninfo).map_err(|_| Error::Invalid("the PostgreSQL connection string is not valid".into()))?;
        config.application_name("ReportCraft");
        // Like libpq: a password left out of the connection string may come from PGPASSWORD, so it
        // needn't appear on a command line.
        if config.get_password().is_none()
            && let Ok(password) = std::env::var("PGPASSWORD")
        {
            config.password(password);
        }
        if config.get_connect_timeout().is_none() {
            config.connect_timeout(CONNECT_TIMEOUT);
        }
        config.connect(NoTls).map_err(|e| Error::Io(format!("cannot connect to PostgreSQL: {}", message(&e))))
    }

    /// A read-only transaction with the time limit set; dropping it rolls back.
    fn begin<'c>(&self, client: &'c mut Client) -> Result<Transaction<'c>> {
        let mut tx = client.build_transaction().read_only(true).start().map_err(|e| query_error(&e))?;
        let ms = self.timeout.as_millis().clamp(1, 3_600_000);
        tx.batch_execute(&format!("SET LOCAL statement_timeout = {ms}; SET LOCAL lock_timeout = {ms}")).map_err(|e| query_error(&e))?;
        Ok(tx)
    }
}

/// The server's message when there is one, without connection details.
fn message(e: &postgres::Error) -> String {
    e.as_db_error().map_or_else(|| e.to_string(), |db| db.message().to_owned())
}

fn query_error(e: &postgres::Error) -> Error {
    match e.code() {
        Some(c) if *c == SqlState::QUERY_CANCELED => Error::TooLarge("the query ran past its time limit and was cancelled".into()),
        Some(c) if *c == SqlState::READ_ONLY_SQL_TRANSACTION => Error::Invalid("only queries that read data are allowed (SELECT, WITH …)".into()),
        Some(c) if *c == SqlState::INSUFFICIENT_PRIVILEGE => Error::Invalid(format!("not allowed: {}", message(e))),
        _ => Error::Invalid(format!("query: {}", message(e))),
    }
}

fn field_type(t: &Type) -> FieldType {
    match *t {
        Type::INT2 | Type::INT4 | Type::INT8 | Type::OID => FieldType::Integer,
        Type::FLOAT4 | Type::FLOAT8 | Type::NUMERIC => FieldType::Number,
        Type::BOOL => FieldType::Bool,
        _ => FieldType::Text,
    }
}

/// A value in PostgreSQL's text format, read as its column's type; anything that doesn't fit
/// (`NaN`, `Infinity`, out-of-range numbers) stays text.
fn to_value(raw: Option<&str>, t: FieldType) -> Value {
    let Some(raw) = raw else { return Value::Null };
    match t {
        FieldType::Integer => raw.parse().map_or_else(|_| Value::Text(raw.to_owned()), Value::Int),
        FieldType::Number => raw.parse::<f64>().ok().filter(|n| n.is_finite()).map_or_else(|| Value::Text(raw.to_owned()), Value::Number),
        FieldType::Bool => match raw {
            "t" => Value::Bool(true),
            "f" => Value::Bool(false),
            other => Value::Text(other.to_owned()),
        },
        FieldType::Text => Value::Text(raw.to_owned()),
    }
}

/// Runs one read-only statement through a cursor and collects up to `max_rows` rows.
fn run(tx: &mut Transaction<'_>, sql: &str, binds: &[BindValue], max_rows: usize) -> Result<RowSet> {
    if sql.trim().is_empty() {
        return Err(Error::Invalid("the query is empty".into()));
    }
    let stmt = tx.prepare(sql).map_err(|e| query_error(&e))?;
    if stmt.columns().is_empty() {
        return Err(Error::Invalid("the query returns no rows to show: only queries that read data are allowed (SELECT, WITH …)".into()));
    }
    if stmt.columns().len() > limits::MAX_COLUMNS {
        return Err(Error::TooLarge(format!("more than {} columns", limits::MAX_COLUMNS)));
    }
    let columns: Vec<Column> = stmt.columns().iter().map(|c| Column { name: c.name().to_owned(), field_type: field_type(c.type_()) }).collect();
    if stmt.params().len() != binds.len() {
        return Err(Error::Invalid(format!(
            "the query has {} placeholder(s) but {} value(s): name report parameters as `:name`",
            stmt.params().len(),
            binds.len()
        )));
    }
    let texts: Vec<Option<String>> = binds.iter().map(BindValue::as_text).collect();
    let params: Vec<&(dyn ToSql + Sync)> = texts.iter().map(|t| t as &(dyn ToSql + Sync)).collect();
    tx.execute(&format!("DECLARE {CURSOR} NO SCROLL CURSOR FOR {sql}"), &params).map_err(|e| query_error(&e))?;
    let mut rows: Vec<Vec<Value>> = Vec::new();
    while rows.len() < max_rows {
        let want = BATCH.min(max_rows - rows.len());
        let mut got = 0usize;
        for msg in tx.simple_query(&format!("FETCH {want} FROM {CURSOR}")).map_err(|e| query_error(&e))? {
            if let SimpleQueryMessage::Row(r) = msg {
                got += 1;
                rows.push(columns.iter().enumerate().map(|(i, c)| to_value(r.get(i), c.field_type)).collect());
            }
        }
        if got < want {
            break;
        }
    }
    Ok(RowSet { columns, rows })
}

impl DataSource for PostgresSource {
    fn describe(&self) -> String {
        "PostgreSQL".to_owned()
    }

    fn read(&self, limit: Option<usize>) -> Result<RowSet> {
        if self.query.len() > MAX_QUERY_LEN {
            return Err(Error::TooLarge("query text longer than 1 MiB".into()));
        }
        let max_rows = limit.unwrap_or(limits::MAX_ROWS).min(limits::MAX_ROWS);
        let mut client = self.connect()?;
        let mut tx = self.begin(&mut client)?;
        run(&mut tx, &self.query, &self.binds, max_rows)
    }
}

/// A relation the user may read, with its schema-qualified parts.
struct Relation {
    oid: u32,
    schema: String,
    name: String,
    kind: TableKind,
}

impl Relation {
    /// `name` in the `public` schema, `schema.name` elsewhere.
    fn display(&self) -> String {
        if self.schema == "public" { self.name.clone() } else { format!("{}.{}", self.schema, self.name) }
    }
}

const RELATIONS_SQL: &str = "SELECT c.oid, n.nspname::text, c.relname::text, c.relkind::text
     FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
     WHERE c.relkind IN ('r', 'v', 'm', 'p', 'f')
       AND n.nspname NOT IN ('pg_catalog', 'information_schema') AND n.nspname NOT LIKE 'pg!_%' ESCAPE '!'
       AND has_table_privilege(c.oid, 'SELECT')
     ORDER BY 2, 3";

const COLUMNS_SQL: &str = "SELECT a.attrelid, a.attname::text, format_type(a.atttypid, a.atttypmod), a.atttypid, NOT a.attnotnull
     FROM pg_attribute a WHERE a.attrelid = ANY($1) AND a.attnum > 0 AND NOT a.attisdropped
     ORDER BY a.attrelid, a.attnum";

const PRIMARY_KEYS_SQL: &str = "SELECT con.conrelid, a.attname::text, k.ord
     FROM pg_constraint con CROSS JOIN LATERAL unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord)
     JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum
     WHERE con.contype = 'p' AND con.conrelid = ANY($1)";

const FOREIGN_KEYS_SQL: &str = "SELECT con.oid, con.conrelid, con.confrelid, a.attname::text, af.attname::text
     FROM pg_constraint con CROSS JOIN LATERAL unnest(con.conkey, con.confkey) WITH ORDINALITY AS k(attnum, fattnum, ord)
     JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum
     JOIN pg_attribute af ON af.attrelid = con.confrelid AND af.attnum = k.fattnum
     WHERE con.contype = 'f' AND con.conrelid = ANY($1)
     ORDER BY con.oid, k.ord";

fn relations(tx: &mut Transaction<'_>) -> Result<Vec<Relation>> {
    let rows = tx.query(RELATIONS_SQL, &[]).map_err(|e| query_error(&e))?;
    if rows.len() > MAX_TABLES {
        return Err(Error::TooLarge(format!("more than {MAX_TABLES} tables and views")));
    }
    rows.iter()
        .map(|r| {
            let kind: String = r.try_get(3).map_err(|e| query_error(&e))?;
            Ok(Relation {
                oid: r.try_get(0).map_err(|e| query_error(&e))?,
                schema: r.try_get(1).map_err(|e| query_error(&e))?,
                name: r.try_get(2).map_err(|e| query_error(&e))?,
                kind: if kind == "v" || kind == "m" { TableKind::View } else { TableKind::Table },
            })
        })
        .collect()
}

impl SchemaSource for PostgresSource {
    fn schema(&self) -> Result<Schema> {
        let mut client = self.connect()?;
        let mut tx = self.begin(&mut client)?;
        let rels = relations(&mut tx)?;
        let oids: Vec<u32> = rels.iter().map(|r| r.oid).collect();
        let index = |oid: u32| rels.iter().position(|r| r.oid == oid);
        let mut tables: Vec<Table> =
            rels.iter().map(|r| Table { name: r.display(), kind: r.kind, columns: Vec::new(), foreign_keys: Vec::new() }).collect();

        for r in tx.query(COLUMNS_SQL, &[&oids]).map_err(|e| query_error(&e))? {
            let rel: u32 = r.try_get(0).map_err(|e| query_error(&e))?;
            let type_oid: u32 = r.try_get(3).map_err(|e| query_error(&e))?;
            let Some(table) = index(rel).and_then(|i| tables.get_mut(i)) else { continue };
            if table.columns.len() >= limits::MAX_COLUMNS {
                return Err(Error::TooLarge(format!("`{}` has more than {} columns", table.name, limits::MAX_COLUMNS)));
            }
            table.columns.push(ColumnInfo {
                name: r.try_get(1).map_err(|e| query_error(&e))?,
                declared_type: r.try_get(2).map_err(|e| query_error(&e))?,
                field_type: Type::from_oid(type_oid).as_ref().map_or(FieldType::Text, field_type),
                nullable: r.try_get(4).map_err(|e| query_error(&e))?,
                primary_key: None,
            });
        }
        for r in tx.query(PRIMARY_KEYS_SQL, &[&oids]).map_err(|e| query_error(&e))? {
            let rel: u32 = r.try_get(0).map_err(|e| query_error(&e))?;
            let column: String = r.try_get(1).map_err(|e| query_error(&e))?;
            let position: i64 = r.try_get(2).map_err(|e| query_error(&e))?;
            if let Some(c) = index(rel).and_then(|i| tables.get_mut(i)).and_then(|t| t.columns.iter_mut().find(|c| c.name == column)) {
                c.primary_key = u32::try_from(position).ok();
            }
        }
        let mut last: Option<u32> = None;
        for r in tx.query(FOREIGN_KEYS_SQL, &[&oids]).map_err(|e| query_error(&e))? {
            let constraint: u32 = r.try_get(0).map_err(|e| query_error(&e))?;
            let rel: u32 = r.try_get(1).map_err(|e| query_error(&e))?;
            let target: u32 = r.try_get(2).map_err(|e| query_error(&e))?;
            let from: String = r.try_get(3).map_err(|e| query_error(&e))?;
            let to: String = r.try_get(4).map_err(|e| query_error(&e))?;
            // A key to a table the user cannot read is kept (the relationship is part of the schema) but
            // its target is shown only by number, not by name.
            let target_name = match index(target).and_then(|i| rels.get(i)) {
                Some(t) => t.display(),
                None => format!("<table {target}>"),
            };
            let Some(table) = index(rel).and_then(|i| tables.get_mut(i)) else { continue };
            match table.foreign_keys.last_mut() {
                Some(fk) if last == Some(constraint) => {
                    fk.columns.push(from);
                    fk.references_columns.push(to);
                }
                _ => table.foreign_keys.push(ForeignKey { columns: vec![from], references_table: target_name, references_columns: vec![to] }),
            }
            last = Some(constraint);
        }
        Ok(Schema { tables })
    }

    fn preview(&self, table: &str, limit: usize) -> Result<RowSet> {
        let mut client = self.connect()?;
        let mut tx = self.begin(&mut client)?;
        let rels = relations(&mut tx)?;
        let rel = rels.iter().find(|r| r.display().eq_ignore_ascii_case(table)).ok_or_else(|| Error::NotFound(format!("table or view `{table}`")))?;
        let limit = limit.min(limits::MAX_ROWS);
        let sql = format!("SELECT * FROM {}.{} LIMIT {limit}", quote_identifier(&rel.schema), quote_identifier(&rel.name));
        run(&mut tx, &sql, &[], limit)
    }
}

/// `"name"` with inner quotes doubled: safe for any identifier.
fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_follow_column_types() {
        assert_eq!(to_value(Some("42"), FieldType::Integer), Value::Int(42));
        assert_eq!(to_value(Some("720.50"), FieldType::Number), Value::Number(720.5));
        assert_eq!(to_value(Some("NaN"), FieldType::Number), Value::Text("NaN".into()));
        assert_eq!(to_value(Some("t"), FieldType::Bool), Value::Bool(true));
        assert_eq!(to_value(None, FieldType::Text), Value::Null);
        assert_eq!(field_type(&Type::NUMERIC), FieldType::Number);
        assert_eq!(field_type(&Type::DATE), FieldType::Text);
    }

    #[test]
    fn bad_connection_strings_never_echo_their_content() {
        let e = PostgresSource::new("postgresql://u:s3cret@[bad", "SELECT 1").read(None).unwrap_err();
        assert!(!e.to_string().contains("s3cret"), "{e}");
        assert!(!format!("{:?}", PostgresSource::new("password=s3cret", "SELECT 1")).contains("s3cret"));
    }

    #[test]
    fn identifiers_are_quoted() {
        assert_eq!(quote_identifier(r#"a"b"#), r#""a""b""#);
    }
}
