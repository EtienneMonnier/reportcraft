//! ReportCraft SQL Server source (L3, native only, pure Rust): reads rows with one query and
//! describes the database's schema, read-only.
//!
//! The connection string is ADO.NET's (`Server=tcp:host,1433;User Id=…;Password=…;Database=…;
//! TrustServerCertificate=true`); a password left out may come from `SQLCMDPASSWORD`, as with
//! `sqlcmd`, and the string is never written to errors or logs. Connections are encrypted with
//! rustls when the server offers it; the certificate is checked against the system's authorities
//! unless `TrustServerCertificate=true`.
//!
//! SQL Server has no read-only transaction, so a report query must pass a scan of its text (one
//! statement, `SELECT` or `WITH`, no word that writes: see `guard`) and then runs inside a
//! transaction that is rolled back. A login with only the `db_datareader` role is the guarantee.
//! Rows are streamed and capped in number and memory; the time limit and a raised `Cancel` flag
//! stop the statement on the server (a TDS attention).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod ado;
mod guard;

use std::future::Future;
use std::time::Duration;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use futures_util::TryStreamExt;
use reportcraft_core::{Cancel, Column, Error, FieldType, Result, RowSet, Value, limits};
use reportcraft_data::{BindValue, ColumnInfo, DataSource, ForeignKey, Schema, SchemaSource, Table, TableKind};
use tiberius::{AuthMethod, Client, ColumnData, ColumnType, Config, FromSql, Query, QueryItem};
use tokio::net::TcpStream;
// tokio's clock: the crate is native only, and std's `Instant::now` is kept off wasm crates.
use tokio::time::Instant;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

type Conn = Client<Compat<TcpStream>>;

/// How long a query may run by default.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// The longest time limit a query may be given (a larger one is cut to this).
const MAX_TIMEOUT: Duration = Duration::from_secs(24 * 3_600);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long stopping a statement or rolling back may take before the connection is just closed
/// (which also rolls back).
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_QUERY_LEN: usize = 1024 * 1024;
const MAX_TABLES: usize = 10_000;
/// How often a running query looks at its cancel flag and its time limit.
const WATCH_INTERVAL: Duration = Duration::from_millis(50);

/// A query over one SQL Server database.
#[derive(Clone)]
pub struct SqlServerSource {
    /// ADO.NET connection string. Never printed.
    connection: String,
    /// A password given apart from the connection string (a saved connection's). Never printed.
    password: Option<String>,
    query: String,
    binds: Vec<BindValue>,
    timeout: Duration,
    cancel: Cancel,
    max_bytes: usize,
}

impl std::fmt::Debug for SqlServerSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqlServerSource")
            .field("query", &self.query)
            .field("binds", &self.binds.len())
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl SqlServerSource {
    pub fn new(connection: impl Into<String>, query: impl Into<String>) -> Self {
        Self {
            connection: connection.into(),
            password: None,
            query: query.into(),
            binds: Vec::new(),
            timeout: DEFAULT_TIMEOUT,
            cancel: Cancel::default(),
            max_bytes: limits::DEFAULT_DATA_BYTES,
        }
    }

    /// A source for schema discovery and previews only (no report query).
    pub fn database(connection: impl Into<String>) -> Self {
        Self::new(connection, "")
    }

    /// The same database with this report query.
    pub fn with_query(mut self, query: impl Into<String>) -> Self {
        self.query = query.into();
        self
    }

    /// Stops the running statement on the server when `cancel` is raised (from another thread).
    pub fn with_cancel(mut self, cancel: Cancel) -> Self {
        self.cancel = cancel;
        self
    }

    /// Stops reading once the rows take about this much memory (at most `limits::MAX_DATA_BYTES`).
    pub fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes.min(limits::MAX_DATA_BYTES);
        self
    }

    /// Values for the query's `@P1`, `@P2`… placeholders, sent with their own type (see
    /// `reportcraft_data::bind`). Values are never printed.
    pub fn with_binds(mut self, binds: Vec<BindValue>) -> Self {
        self.binds = binds;
        self
    }

    /// Logs in with this password (and the connection string's user) instead of one in the string:
    /// any character goes, where the string's own syntax takes ASCII only.
    pub fn with_password(mut self, password: Option<String>) -> Self {
        self.password = password;
        self
    }

    /// Stops the query once it has run this long.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout.min(MAX_TIMEOUT);
        self
    }

    /// Where the source connects, for logs and listings: `user@host:port/database`, never the
    /// password or any other option. `SQL Server` when the connection string isn't valid.
    pub fn target(&self) -> String {
        let Ok(config) = Config::from_ado_string(&self.connection) else { return "SQL Server".to_owned() };
        let pairs = ado::pairs(&self.connection);
        let user = ado::get(&pairs, ado::USER).filter(|u| !u.is_empty()).map(|u| format!("{u}@")).unwrap_or_default();
        format!("{user}{}/{}", config.get_addr(), ado::get(&pairs, ado::DATABASE).unwrap_or_default())
    }

    fn config(&self) -> Result<Config> {
        let mut config = Config::from_ado_string(&self.connection).map_err(|_| {
            Error::Invalid("the SQL Server connection string is not valid (ADO.NET form: Server=tcp:host,1433;User Id=…;Database=…)".into())
        })?;
        config.application_name("ReportCraft");
        let pairs = ado::pairs(&self.connection);
        // Like sqlcmd: a password left out of the connection string may come from SQLCMDPASSWORD,
        // so it needn't appear on a command line.
        let password =
            self.password.clone().or_else(|| if ado::get(&pairs, ado::PASSWORD).is_none() { std::env::var("SQLCMDPASSWORD").ok() } else { None });
        if let Some(password) = password
            && let Some(user) = ado::get(&pairs, ado::USER)
        {
            config.authentication(AuthMethod::sql_server(user, password));
        }
        Ok(config)
    }

    /// Connects, opens the transaction that is never committed, runs `work`, then stops whatever
    /// is still running and rolls back.
    fn session<T>(&self, work: impl AsyncFnOnce(&mut Conn, &mut bool, Instant) -> Result<T>) -> Result<T> {
        self.cancel.check("the query")?;
        let config = self.config()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| Error::Io(format!("cannot start the SQL Server client: {e}")))?;
        runtime.block_on(async {
            let now = Instant::now();
            // The time limit is capped, so the sum fits; were it not, the query would just stop at once.
            let deadline = now.checked_add(self.timeout).unwrap_or(now);
            let mut client = connect(config).await?;
            begin(&mut client, self.timeout).await?;
            let mut running = false;
            let result = work(&mut client, &mut running, deadline).await;
            if running {
                // Best effort: closing the connection stops and rolls back the statement anyway.
                let _ = tokio::time::timeout(CLEANUP_TIMEOUT, client.cancel_query()).await;
            }
            let _ = tokio::time::timeout(CLEANUP_TIMEOUT, client.simple_query("IF @@TRANCOUNT > 0 ROLLBACK")).await;
            result
        })
    }
}

async fn connect(config: Config) -> Result<Conn> {
    let addr = config.get_addr();
    let connecting = async {
        let tcp = TcpStream::connect(addr).await.map_err(|e| Error::Io(format!("cannot connect to SQL Server: {e}")))?;
        let _ = tcp.set_nodelay(true);
        Client::connect(config, tcp.compat_write()).await.map_err(|e| Error::Io(format!("cannot connect to SQL Server: {}", message(&e))))
    };
    tokio::time::timeout(CONNECT_TIMEOUT, connecting)
        .await
        .map_err(|_| Error::Io(format!("cannot connect to SQL Server: no answer within {} s", CONNECT_TIMEOUT.as_secs())))?
}

/// The transaction everything runs in (rolled back at the end), with the lock wait limited.
async fn begin(client: &mut Conn, timeout: Duration) -> Result<()> {
    let ms = timeout.as_millis().clamp(1, 3_600_000);
    // A plain batch: inside a parameterised call (`sp_executesql`) the transaction would have to end
    // before the call does.
    let started = async { client.simple_query(format!("SET XACT_ABORT ON; SET LOCK_TIMEOUT {ms}; BEGIN TRANSACTION")).await?.into_results().await };
    started.await.map_err(|e| query_error(&e))?;
    Ok(())
}

/// `work`, stopped when the cancel flag is raised or the deadline passes.
async fn watched<T>(work: impl Future<Output = T>, cancel: &Cancel, deadline: Instant) -> Result<T> {
    let mut work = std::pin::pin!(work);
    loop {
        if let Ok(done) = tokio::time::timeout(WATCH_INTERVAL, work.as_mut()).await {
            return Ok(done);
        }
        if cancel.is_cancelled() {
            return Err(Error::Cancelled("the query was stopped on request".into()));
        }
        if Instant::now() >= deadline {
            return Err(Error::TooLarge("the query ran past its time limit and was cancelled".into()));
        }
    }
}

/// The server's message when there is one, without connection details.
fn message(e: &tiberius::error::Error) -> String {
    match e {
        tiberius::error::Error::Server(t) => t.message().to_owned(),
        other => other.to_string(),
    }
}

fn query_error(e: &tiberius::error::Error) -> Error {
    match e {
        tiberius::error::Error::Server(t) => match t.code() {
            // Permission denied on an object, a column, a database.
            229 | 230 | 262 | 297 | 300 | 916 => Error::Invalid(format!("not allowed: {}", t.message())),
            1222 => Error::TooLarge("the query waited past its time limit for a lock and was cancelled".into()),
            3906 => Error::Invalid("only queries that read data are allowed (SELECT, WITH …)".into()),
            _ => Error::Invalid(format!("query: {}", t.message())),
        },
        other => Error::Io(format!("SQL Server: {other}")),
    }
}

fn field_type(t: ColumnType) -> FieldType {
    match t {
        ColumnType::Int1 | ColumnType::Int2 | ColumnType::Int4 | ColumnType::Int8 | ColumnType::Intn => FieldType::Integer,
        ColumnType::Float4
        | ColumnType::Float8
        | ColumnType::Floatn
        | ColumnType::Money
        | ColumnType::Money4
        | ColumnType::Decimaln
        | ColumnType::Numericn => FieldType::Number,
        ColumnType::Bit | ColumnType::Bitn => FieldType::Bool,
        _ => FieldType::Text,
    }
}

/// The field type of a declared (system) type name.
fn field_type_named(name: &str) -> FieldType {
    match name {
        "tinyint" | "smallint" | "int" | "bigint" => FieldType::Integer,
        "real" | "float" | "decimal" | "numeric" | "money" | "smallmoney" => FieldType::Number,
        "bit" => FieldType::Bool,
        _ => FieldType::Text,
    }
}

/// A column's type as written in `CREATE TABLE`: `nvarchar(40)`, `decimal(10,2)`, `varchar(max)`.
fn declared(name: &str, max_length: i64, precision: i64, scale: i64) -> String {
    let length = |unit: i64| if max_length == -1 { "max".to_owned() } else { (max_length / unit).to_string() };
    match name {
        "varchar" | "char" | "varbinary" | "binary" => format!("{name}({})", length(1)),
        "nvarchar" | "nchar" => format!("{name}({})", length(2)),
        "decimal" | "numeric" => format!("{name}({precision},{scale})"),
        "datetime2" | "time" | "datetimeoffset" => format!("{name}({scale})"),
        _ => name.to_owned(),
    }
}

fn number(n: f64) -> Value {
    if n.is_finite() { Value::Number(n) } else { Value::Text(n.to_string()) }
}

/// A value as its column's type; dates and times as ISO text (`2026-10-01 08:30:00`), as the
/// other databases give them; binary data as `0x…`.
fn to_value(data: &ColumnData<'static>) -> Value {
    match data {
        ColumnData::U8(Some(v)) => Value::Int(i64::from(*v)),
        ColumnData::I16(Some(v)) => Value::Int(i64::from(*v)),
        ColumnData::I32(Some(v)) => Value::Int(i64::from(*v)),
        ColumnData::I64(Some(v)) => Value::Int(*v),
        ColumnData::F32(Some(v)) => number(f64::from(*v)),
        ColumnData::F64(Some(v)) => number(*v),
        ColumnData::Bit(Some(b)) => Value::Bool(*b),
        ColumnData::String(Some(s)) => Value::Text(s.to_string()),
        ColumnData::Guid(Some(g)) => Value::Text(g.to_string().to_uppercase()),
        ColumnData::Binary(Some(b)) => Value::Text(b.iter().fold("0x".to_owned(), |mut s, byte| {
            s.push_str(&format!("{byte:02X}"));
            s
        })),
        ColumnData::Numeric(Some(n)) => number(f64::from(*n)),
        ColumnData::Xml(Some(x)) => Value::Text(x.to_string()),
        ColumnData::DateTime(Some(_)) | ColumnData::SmallDateTime(Some(_)) | ColumnData::DateTime2(Some(_)) => text_of(NaiveDateTime::from_sql(data)),
        ColumnData::Date(Some(_)) => text_of(NaiveDate::from_sql(data)),
        ColumnData::Time(Some(_)) => text_of(NaiveTime::from_sql(data)),
        ColumnData::DateTimeOffset(Some(_)) => text_of(DateTime::<FixedOffset>::from_sql(data)),
        _ => Value::Null,
    }
}

fn text_of<T: ToString>(v: tiberius::Result<Option<T>>) -> Value {
    v.ok().flatten().map_or(Value::Null, |t| Value::Text(t.to_string()))
}

fn bind<'a>(q: &mut Query<'a>, value: &BindValue) {
    match value {
        BindValue::Null => q.bind(Option::<String>::None),
        BindValue::Text(t) => q.bind(t.clone()),
        BindValue::Integer(i) => q.bind(*i),
        BindValue::Number(n) => q.bind(*n),
        BindValue::Boolean(b) => q.bind(*b),
        BindValue::Date(d) => q.bind(*d),
    }
}

/// Limits on one read.
struct Read<'s> {
    max_rows: usize,
    max_bytes: usize,
    cancel: &'s Cancel,
    deadline: Instant,
}

/// Runs one statement and streams up to `max_rows` rows; `running` stays set while the server may
/// still be sending.
async fn rows(client: &mut Conn, sql: &str, binds: &[BindValue], limits_: &Read<'_>, running: &mut bool) -> Result<RowSet> {
    let (cancel, deadline) = (limits_.cancel, limits_.deadline);
    let mut q = Query::new(sql.to_owned());
    for b in binds {
        bind(&mut q, b);
    }
    *running = true;
    let mut stream = watched(q.query(client), cancel, deadline).await?.map_err(|e| query_error(&e))?;
    let columns: Vec<Column> = match watched(stream.columns(), cancel, deadline).await?.map_err(|e| query_error(&e))? {
        Some(cs) => cs.iter().map(|c| Column { name: c.name().to_owned(), field_type: field_type(c.column_type()) }).collect(),
        None => {
            return Err(Error::Invalid("the query returns no rows to show: only queries that read data are allowed (SELECT, WITH …)".into()));
        }
    };
    if columns.len() > limits::MAX_COLUMNS {
        return Err(Error::TooLarge(format!("more than {} columns", limits::MAX_COLUMNS)));
    }
    let mut rows: Vec<Vec<Value>> = Vec::new();
    let mut bytes = 0usize;
    while rows.len() < limits_.max_rows {
        match watched(stream.try_next(), cancel, deadline).await?.map_err(|e| query_error(&e))? {
            None => {
                *running = false;
                break;
            }
            Some(QueryItem::Metadata(m)) if m.result_index() > 0 => break,
            Some(QueryItem::Metadata(_)) => {}
            Some(QueryItem::Row(r)) => {
                if r.result_index() > 0 {
                    break;
                }
                let row: Vec<Value> = r.cells().map(|(_, d)| to_value(d)).collect();
                bytes = bytes.saturating_add(std::mem::size_of::<Vec<Value>>() + row.iter().map(Value::size).sum::<usize>());
                if bytes > limits_.max_bytes {
                    return Err(Error::TooLarge(format!(
                        "the rows take more than {} MiB after {} rows; narrow the query or raise the memory limit",
                        limits_.max_bytes >> 20,
                        rows.len()
                    )));
                }
                rows.push(row);
            }
        }
    }
    Ok(RowSet { columns, rows })
}

impl DataSource for SqlServerSource {
    fn describe(&self) -> String {
        "SQL Server".to_owned()
    }

    fn read(&self, limit: Option<usize>) -> Result<RowSet> {
        if self.query.len() > MAX_QUERY_LEN {
            return Err(Error::TooLarge("query text longer than 1 MiB".into()));
        }
        let placeholders = guard::check(&self.query)?;
        if placeholders != self.binds.len() {
            return Err(Error::Invalid(format!(
                "the query has {placeholders} placeholder(s) but {} value(s): name report parameters as `:name`",
                self.binds.len()
            )));
        }
        let max_rows = limit.unwrap_or(limits::MAX_ROWS).min(limits::MAX_ROWS);
        self.session(async |client, running, deadline| {
            let read = Read { max_rows, max_bytes: self.max_bytes, cancel: &self.cancel, deadline };
            rows(client, &self.query, &self.binds, &read, running).await
        })
    }
}

/// A table or view the user may read.
struct Relation {
    id: i64,
    schema: String,
    name: String,
    kind: TableKind,
}

impl Relation {
    /// `name` in the `dbo` schema, `schema.name` elsewhere.
    fn display(&self) -> String {
        if self.schema == "dbo" { self.name.clone() } else { format!("{}.{}", self.schema, self.name) }
    }
}

const RELATIONS_SQL: &str = "SELECT o.object_id, s.name, o.name, RTRIM(o.type)
     FROM sys.objects o JOIN sys.schemas s ON s.schema_id = o.schema_id
     WHERE o.type IN ('U', 'V') AND o.is_ms_shipped = 0
       AND HAS_PERMS_BY_NAME(QUOTENAME(s.name) + '.' + QUOTENAME(o.name), 'OBJECT', 'SELECT') = 1
     ORDER BY s.name, o.name";

const COLUMNS_SQL: &str = "SELECT c.object_id, c.name, TYPE_NAME(c.user_type_id), TYPE_NAME(c.system_type_id),
            CAST(c.max_length AS int), CAST(c.precision AS int), CAST(c.scale AS int), c.is_nullable
     FROM sys.columns c JOIN sys.objects o ON o.object_id = c.object_id
     WHERE o.type IN ('U', 'V') AND o.is_ms_shipped = 0
     ORDER BY c.object_id, c.column_id";

const PRIMARY_KEYS_SQL: &str = "SELECT ic.object_id, c.name, CAST(ic.key_ordinal AS int)
     FROM sys.indexes i
     JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id
     JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id
     WHERE i.is_primary_key = 1";

const FOREIGN_KEYS_SQL: &str = "SELECT f.constraint_object_id, f.parent_object_id, f.referenced_object_id, pc.name, rc.name
     FROM sys.foreign_key_columns f
     JOIN sys.columns pc ON pc.object_id = f.parent_object_id AND pc.column_id = f.parent_column_id
     JOIN sys.columns rc ON rc.object_id = f.referenced_object_id AND rc.column_id = f.referenced_column_id
     ORDER BY f.constraint_object_id, f.constraint_column_id";

/// All rows of a catalog query, as values.
async fn catalog(client: &mut Conn, sql: &str, cancel: &Cancel, deadline: Instant) -> Result<Vec<Vec<Value>>> {
    let stream = watched(client.simple_query(sql), cancel, deadline).await?.map_err(|e| query_error(&e))?;
    let rows = watched(stream.into_first_result(), cancel, deadline).await?.map_err(|e| query_error(&e))?;
    Ok(rows.iter().map(|r| r.cells().map(|(_, d)| to_value(d)).collect()).collect())
}

fn int(row: &[Value], i: usize) -> i64 {
    match row.get(i) {
        Some(Value::Int(n)) => *n,
        _ => 0,
    }
}

fn text(row: &[Value], i: usize) -> String {
    match row.get(i) {
        Some(Value::Text(t)) => t.clone(),
        _ => String::new(),
    }
}

async fn relations(client: &mut Conn, cancel: &Cancel, deadline: Instant) -> Result<Vec<Relation>> {
    let rows = catalog(client, RELATIONS_SQL, cancel, deadline).await?;
    if rows.len() > MAX_TABLES {
        return Err(Error::TooLarge(format!("more than {MAX_TABLES} tables and views")));
    }
    Ok(rows
        .iter()
        .map(|r| Relation {
            id: int(r, 0),
            schema: text(r, 1),
            name: text(r, 2),
            kind: if text(r, 3) == "V" { TableKind::View } else { TableKind::Table },
        })
        .collect())
}

/// Adds the foreign keys of `rows` (from `FOREIGN_KEYS_SQL`) to their tables.
fn add_foreign_keys(rows: &[Vec<Value>], rels: &[Relation], tables: &mut [Table]) {
    let index = |id: i64| rels.iter().position(|r| r.id == id);
    let mut last: Option<i64> = None;
    for r in rows {
        let constraint = int(r, 0);
        let (from, to) = (text(r, 3), text(r, 4));
        // A key to a table the user cannot read is kept (the relationship is part of the schema) but
        // its target is shown only by number, not by name.
        let target = int(r, 2);
        let target_name = index(target).and_then(|i| rels.get(i)).map_or_else(|| format!("<table {target}>"), Relation::display);
        let Some(table) = index(int(r, 1)).and_then(|i| tables.get_mut(i)) else { continue };
        match table.foreign_keys.last_mut() {
            Some(fk) if last == Some(constraint) => {
                fk.columns.push(from);
                fk.references_columns.push(to);
            }
            _ => table.foreign_keys.push(ForeignKey { columns: vec![from], references_table: target_name, references_columns: vec![to] }),
        }
        last = Some(constraint);
    }
}

impl SchemaSource for SqlServerSource {
    fn schema(&self) -> Result<Schema> {
        self.session(async |client, _, deadline| {
            let cancel = &self.cancel;
            let rels = relations(client, cancel, deadline).await?;
            let index = |id: i64| rels.iter().position(|r| r.id == id);
            let mut tables: Vec<Table> =
                rels.iter().map(|r| Table { name: r.display(), kind: r.kind, columns: Vec::new(), foreign_keys: Vec::new() }).collect();
            for r in catalog(client, COLUMNS_SQL, cancel, deadline).await? {
                let Some(table) = index(int(&r, 0)).and_then(|i| tables.get_mut(i)) else { continue };
                if table.columns.len() >= limits::MAX_COLUMNS {
                    return Err(Error::TooLarge(format!("`{}` has more than {} columns", table.name, limits::MAX_COLUMNS)));
                }
                table.columns.push(ColumnInfo {
                    name: text(&r, 1),
                    declared_type: declared(&text(&r, 2), int(&r, 4), int(&r, 5), int(&r, 6)),
                    field_type: field_type_named(&text(&r, 3)),
                    nullable: matches!(r.get(7), Some(Value::Bool(true))),
                    primary_key: None,
                });
            }
            for r in catalog(client, PRIMARY_KEYS_SQL, cancel, deadline).await? {
                let column = text(&r, 1);
                if let Some(c) = index(int(&r, 0)).and_then(|i| tables.get_mut(i)).and_then(|t| t.columns.iter_mut().find(|c| c.name == column)) {
                    c.primary_key = u32::try_from(int(&r, 2)).ok();
                }
            }
            let keys = catalog(client, FOREIGN_KEYS_SQL, cancel, deadline).await?;
            add_foreign_keys(&keys, &rels, &mut tables);
            Ok(Schema { tables })
        })
    }

    fn preview(&self, table: &str, limit: usize) -> Result<RowSet> {
        let limit = limit.min(limits::MAX_ROWS);
        self.session(async |client, running, deadline| {
            let rels = relations(client, &self.cancel, deadline).await?;
            let rel =
                rels.iter().find(|r| r.display().eq_ignore_ascii_case(table)).ok_or_else(|| Error::NotFound(format!("table or view `{table}`")))?;
            let sql = format!("SELECT TOP ({limit}) * FROM {}.{}", quote_identifier(&rel.schema), quote_identifier(&rel.name));
            let read = Read { max_rows: limit, max_bytes: self.max_bytes, cancel: &self.cancel, deadline };
            rows(client, &sql, &[], &read, running).await
        })
    }
}

/// `[name]` with inner `]` doubled: safe for any identifier.
fn quote_identifier(name: &str) -> String {
    format!("[{}]", name.replace(']', "]]"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_follow_column_types() {
        assert_eq!(to_value(&ColumnData::I32(Some(42))), Value::Int(42));
        assert_eq!(to_value(&ColumnData::U8(Some(7))), Value::Int(7));
        assert_eq!(to_value(&ColumnData::F64(Some(f64::NAN))), Value::Text("NaN".into()));
        assert_eq!(to_value(&ColumnData::Numeric(Some(tiberius::numeric::Numeric::new_with_scale(72050, 2)))), Value::Number(720.5));
        assert_eq!(to_value(&ColumnData::Bit(Some(true))), Value::Bool(true));
        assert_eq!(to_value(&ColumnData::String(None)), Value::Null);
        assert_eq!(to_value(&ColumnData::Binary(Some(vec![0x0f, 0xa0].into()))), Value::Text("0x0FA0".into()));
        assert_eq!(field_type(ColumnType::Decimaln), FieldType::Number);
        assert_eq!(field_type(ColumnType::Daten), FieldType::Text);
        assert_eq!(field_type_named("smallmoney"), FieldType::Number);
        assert_eq!(declared("nvarchar", 80, 0, 0), "nvarchar(40)");
        assert_eq!(declared("varchar", -1, 0, 0), "varchar(max)");
        assert_eq!(declared("decimal", 9, 10, 2), "decimal(10,2)");
        assert_eq!(declared("int", 4, 10, 0), "int");
    }

    #[test]
    fn connection_strings_never_echo_their_content() {
        let bad = SqlServerSource::new("Server=db;Password='s3cret", "SELECT 1");
        let e = bad.read(None).unwrap_err();
        assert!(!e.to_string().contains("s3cret"), "{e}");
        assert!(!format!("{bad:?}").contains("s3cret"));
        let ok = SqlServerSource::database("Server=tcp:db.local,1433;User Id=rc;Password=s3cret;Database=rent;TrustServerCertificate=true");
        assert_eq!(ok.target(), "rc@db.local:1433/rent");
        assert_eq!(SqlServerSource::database("Server=db;Password='s3cret").target(), "SQL Server");
    }

    #[test]
    fn writing_queries_are_refused_before_connecting() {
        let e = SqlServerSource::new("Server=nowhere.invalid", "DELETE FROM t").read(None).unwrap_err();
        assert!(e.to_string().contains("only queries that read"), "{e}");
        let e = SqlServerSource::new("Server=nowhere.invalid", "SELECT * FROM t WHERE a = @P1").read(None).unwrap_err();
        assert!(e.to_string().contains("1 placeholder(s) but 0"), "{e}");
    }

    #[test]
    fn identifiers_are_quoted() {
        assert_eq!(quote_identifier("a]b"), "[a]]b]");
    }
}
