//! The command registry. A command is an id, a label, its menu path, a parameter description
//! and `run`. Ids follow the menus (`file.open`, `data.loadCsv`, `report.render`) plus primitives.

use reportcraft_core::RowSet;
use reportcraft_data::{CsvSource, DataSource};
use reportcraft_layout::{Params, to_html};
use reportcraft_model::Report;
use serde_json::{Value as Json, json};

use crate::{EngineError, EngineResult, Session, sample};

pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub menu: &'static str,
    /// Parameters, as shown by `reportcraft-cli commands` and the MCP tool list.
    pub params: &'static str,
    pub run: fn(&mut Session, &Json) -> EngineResult<Json>,
}

static COMMANDS: &[CommandSpec] = &[
    CommandSpec { id: "file.new", label: "New report", menu: "File", params: r#"{"title"?: string}"#, run: file_new },
    CommandSpec { id: "file.openSample", label: "Open sample", menu: "File", params: r#"{"name": "rentals"}"#, run: file_open_sample },
    CommandSpec {
        id: "report.load",
        label: "Load report definition",
        menu: "File",
        params: r#"{"json": string (a report definition)}"#,
        run: report_load,
    },
    CommandSpec { id: "report.get", label: "Report definition", menu: "Report", params: "{}", run: report_get },
    CommandSpec {
        id: "data.loadCsv",
        label: "Load CSV data",
        menu: "Data",
        params: r#"{"csv": string (header row first), "dataset"?: string (name it for a subreport instead of the report's own data)}"#,
        run: data_load_csv,
    },
    CommandSpec {
        id: "data.loadSqlite",
        label: "Load SQLite data",
        menu: "Data",
        params: r#"{"path": string (database file, opened read-only), "query": string (one SELECT; report parameters as :name, :range.from, :range.to, IN (:list)), "params"?: {name: value or [values]}, "limit"?: integer, "timeoutSeconds"?: integer (default 30, at most 3600), "maxMegabytes"?: integer (memory the rows may take, default 1024, at most 16384), "cacheSeconds"?: integer (reuse the same query's rows read less than this long ago, at most 86400), "dataset"?: string (name it for a subreport instead of the report's own data)}"#,
        run: data_load_sqlite,
    },
    CommandSpec {
        id: "data.describeSqlite",
        label: "Describe SQLite database",
        menu: "Data",
        params: r#"{"path": string (database file, opened read-only), "table"?: string (only this table or view)}"#,
        run: data_describe_sqlite,
    },
    CommandSpec {
        id: "data.previewSqlite",
        label: "Preview SQLite table",
        menu: "Data",
        params: r#"{"path": string, "table": string (table or view), "limit"?: integer (default 20, at most 1000)}"#,
        run: data_preview_sqlite,
    },
    CommandSpec {
        id: "data.loadPostgres",
        label: "Load PostgreSQL data",
        menu: "Data",
        params: r#"{"connection": string (libpq connection string or postgresql:// URL; the password may come from PGPASSWORD), "query": string (one SELECT; report parameters as :name, :range.from, :range.to, IN (:list)), "params"?: {name: value or [values]}, "limit"?: integer, "timeoutSeconds"?: integer (default 30, at most 3600), "maxMegabytes"?: integer (memory the rows may take, default 1024, at most 16384), "cacheSeconds"?: integer (reuse the same query's rows read less than this long ago, at most 86400), "dataset"?: string (name it for a subreport instead of the report's own data)}"#,
        run: data_load_postgres,
    },
    CommandSpec {
        id: "data.describePostgres",
        label: "Describe PostgreSQL database",
        menu: "Data",
        params: r#"{"connection": string, "table"?: string (only this table or view; `schema.table` outside public)}"#,
        run: data_describe_postgres,
    },
    CommandSpec {
        id: "data.previewPostgres",
        label: "Preview PostgreSQL table",
        menu: "Data",
        params: r#"{"connection": string, "table": string, "limit"?: integer (default 20, at most 1000)}"#,
        run: data_preview_postgres,
    },
    CommandSpec { id: "data.datasets", label: "Datasets", menu: "Data", params: "{}", run: data_datasets },
    CommandSpec {
        id: "data.preview",
        label: "Preview data",
        menu: "Data",
        params: r#"{"limit"?: integer (default 20, at most 1000)}"#,
        run: data_preview,
    },
    CommandSpec {
        id: "data.queryLog",
        label: "Query log",
        menu: "Data",
        params: r#"{"limit"?: integer (newest entries, default 50, at most 200)}"#,
        run: data_query_log,
    },
    CommandSpec { id: "data.clearQueryLog", label: "Clear query log", menu: "Data", params: "{}", run: data_clear_query_log },
    CommandSpec { id: "data.clearCache", label: "Clear query cache", menu: "Data", params: "{}", run: data_clear_cache },
    CommandSpec { id: "report.parameters", label: "Report parameters", menu: "Report", params: "{}", run: report_parameters },
    CommandSpec {
        id: "report.validate",
        label: "Check report",
        menu: "Report",
        params: r#"{"json"?: string (a definition to check instead of the open report)}"#,
        run: report_validate,
    },
    CommandSpec {
        id: "report.render",
        label: "Render",
        menu: "Report",
        params: r#"{"format"?: "html" | "json" (default "html"), "params"?: {name: value or [values] (dates YYYY-MM-DD, ranges YYYY-MM-DD..YYYY-MM-DD)}, "timeoutSeconds"?: integer (default 120, at most 3600)}"#,
        run: report_render,
    },
];

pub fn commands() -> &'static [CommandSpec] {
    COMMANDS
}

pub fn find_command(id: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|c| c.id == id)
}

fn bad(command: &str, message: impl Into<String>) -> EngineError {
    EngineError::BadParams { command: command.to_owned(), message: message.into() }
}

fn str_param<'a>(command: &str, params: &'a Json, name: &str) -> EngineResult<&'a str> {
    params.get(name).and_then(Json::as_str).ok_or_else(|| bad(command, format!("`{name}` (string) is required")))
}

fn file_new(s: &mut Session, params: &Json) -> EngineResult<Json> {
    let title = params.get("title").and_then(Json::as_str).unwrap_or("Untitled report");
    s.report = Report::new(title);
    s.rows = Default::default();
    s.refreshed_at = None;
    s.datasets.clear();
    Ok(json!({"title": title}))
}

fn file_open_sample(s: &mut Session, params: &Json) -> EngineResult<Json> {
    let name = params.get("name").and_then(Json::as_str).unwrap_or("rentals");
    let (report, csv) = sample::by_name(name).ok_or_else(|| bad("file.openSample", format!("no sample named `{name}`")))?;
    set_rows(s, CsvSource::new(csv).read(None)?);
    s.report = report;
    Ok(json!({"title": s.report.title, "rows": s.rows.rows.len()}))
}

fn report_load(s: &mut Session, params: &Json) -> EngineResult<Json> {
    let report = Report::from_json(str_param("report.load", params, "json")?)?;
    let title = report.title.clone();
    s.report = report;
    Ok(json!({"title": title}))
}

fn report_get(s: &mut Session, _: &Json) -> EngineResult<Json> {
    serde_json::to_value(&s.report).map_err(|e| EngineError::Internal(e.to_string()))
}

fn data_load_csv(s: &mut Session, params: &Json) -> EngineResult<Json> {
    let rows = CsvSource::new(str_param("data.loadCsv", params, "csv")?).read(None)?;
    if let Some(name) = dataset_of("data.loadCsv", params)? {
        if !s.datasets.contains_key(&name) && s.datasets.len() >= MAX_DATASETS {
            return Err(bad("data.loadCsv", format!("at most {MAX_DATASETS} datasets")));
        }
        let n = rows.rows.len();
        let columns = serde_json::to_value(&rows.columns).unwrap_or_default();
        s.datasets.insert(name.clone(), std::sync::Arc::new(rows));
        return Ok(json!({"dataset": name, "columns": columns, "rows": n}));
    }
    set_rows(s, rows);
    Ok(json!({"columns": s.rows.columns, "rows": s.rows.rows.len()}))
}

/// Most named datasets a session keeps.
const MAX_DATASETS: usize = 64;

/// The `dataset` a load names, checked: letters, digits, `_` or `-`.
fn dataset_of(command: &str, params: &Json) -> EngineResult<Option<String>> {
    match params.get("dataset") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(name)) if !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') => {
            Ok(Some(name.clone()))
        }
        Some(_) => Err(bad(command, "`dataset` must be a name of letters, digits, `_` or `-` (at most 64)")),
    }
}

fn data_datasets(s: &mut Session, _: &Json) -> EngineResult<Json> {
    let list: Vec<Json> = s.datasets.iter().map(|(name, rows)| json!({"name": name, "columns": rows.columns, "rows": rows.rows.len()})).collect();
    Ok(json!({"datasets": list}))
}

/// Hands newly read rows to the report, noting when they were read.
fn set_rows(s: &mut Session, rows: RowSet) {
    s.rows = std::sync::Arc::new(rows);
    s.refreshed_at = now_local();
}

/// The time now, RFC 3339 with the local offset; `None` in the web build (no clock).
#[cfg(not(target_arch = "wasm32"))]
fn now_local() -> Option<String> {
    Some(chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false))
}

#[cfg(target_arch = "wasm32")]
fn now_local() -> Option<String> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn data_load_sqlite(s: &mut Session, params: &Json) -> EngineResult<Json> {
    const CMD: &str = "data.loadSqlite";
    let path = str_param(CMD, params, "path")?;
    let query = str_param(CMD, params, "query")?;
    let (limit, timeout) = limits_of(params);
    let bound = bound_query(s, CMD, query, params, reportcraft_data::Dialect::Sqlite)?;
    let source = reportcraft_sqlite::SqliteSource::new(path, bound.sql.clone())
        .with_binds(bound.values.clone())
        .with_timeout(timeout)
        .with_cancel(s.cancel().clone())
        .with_max_bytes(max_bytes_of(params));
    load_from_database(s, Query { command: CMD, source: "sqlite", target: path.to_owned(), bound: &bound, limit }, params, || source.read(limit))
}

/// A database query about to run, as the log and the cache know it.
#[cfg(not(target_arch = "wasm32"))]
struct Query<'a> {
    command: &'a str,
    source: &'a str,
    target: String,
    bound: &'a reportcraft_data::Bound,
    limit: Option<usize>,
}

/// Runs `read` (or takes its rows from the cache when `cacheSeconds` allows), logs it and hands
/// the rows to the report.
#[cfg(not(target_arch = "wasm32"))]
fn load_from_database(s: &mut Session, q: Query<'_>, params: &Json, read: impl FnOnce() -> reportcraft_core::Result<RowSet>) -> EngineResult<Json> {
    let dataset = dataset_of(q.command, params)?;
    if let Some(name) = &dataset
        && !s.datasets.contains_key(name)
        && s.datasets.len() >= MAX_DATASETS
    {
        return Err(bad(q.command, format!("at most {MAX_DATASETS} datasets")));
    }
    let max_age = params.get("cacheSeconds").and_then(Json::as_u64).unwrap_or(0).min(86_400);
    let key = format!("{}\n{}\n{}\n{:?}\n{:?}\n{}", q.source, q.target, q.bound.sql, q.bound.values, q.limit, max_bytes_of(params));
    let mut logged = Logged::new(q.command, q.source, q.target, q.bound);
    if max_age > 0
        && let Some(hit) = s.cache.get(&key, std::time::Duration::from_secs(max_age))
    {
        logged.entry.from_cache = true;
        logged.entry.rows = Some(hit.rows.rows.len());
        s.query_log.push(logged.entry);
        return Ok(hand_over(s, dataset, hit.rows, Some(hit.refreshed_at), true));
    }
    let rows = std::sync::Arc::new(logged.finish(&mut s.query_log, read())?);
    let at = now_local();
    if max_age > 0 {
        s.cache.put(key, std::sync::Arc::clone(&rows), at.clone().unwrap_or_default());
    }
    Ok(hand_over(s, dataset, rows, at, false))
}

/// Gives loaded rows to the report, or to a named dataset, and says what was loaded.
#[cfg(not(target_arch = "wasm32"))]
fn hand_over(s: &mut Session, dataset: Option<String>, rows: std::sync::Arc<RowSet>, at: Option<String>, from_cache: bool) -> Json {
    let mut out = json!({"columns": rows.columns, "rows": rows.rows.len(), "refreshedAt": at, "fromCache": from_cache});
    match dataset {
        Some(name) => {
            if let Some(map) = out.as_object_mut() {
                map.insert("dataset".into(), Json::String(name.clone()));
            }
            s.datasets.insert(name, rows);
        }
        None => {
            s.rows = rows;
            s.refreshed_at = at;
        }
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
fn data_describe_sqlite(_: &mut Session, params: &Json) -> EngineResult<Json> {
    let path = str_param("data.describeSqlite", params, "path")?;
    describe(&reportcraft_sqlite::SqliteSource::database(path), params)
}

#[cfg(not(target_arch = "wasm32"))]
fn data_preview_sqlite(_: &mut Session, params: &Json) -> EngineResult<Json> {
    const CMD: &str = "data.previewSqlite";
    let path = str_param(CMD, params, "path")?;
    preview(&reportcraft_sqlite::SqliteSource::database(path), CMD, params)
}

/// The schema, or one table of it, as JSON.
#[cfg(not(target_arch = "wasm32"))]
fn describe(source: &dyn reportcraft_data::SchemaSource, params: &Json) -> EngineResult<Json> {
    let schema = source.schema()?;
    let value = match params.get("table").and_then(Json::as_str) {
        Some(name) => {
            let table = schema.table(name).ok_or_else(|| reportcraft_core::Error::NotFound(format!("table or view `{name}`")))?;
            serde_json::to_value(table)
        }
        None => serde_json::to_value(&schema),
    };
    value.map_err(|e| EngineError::Internal(e.to_string()))
}

/// The first rows of one table as JSON.
#[cfg(not(target_arch = "wasm32"))]
fn preview(source: &dyn reportcraft_data::SchemaSource, command: &str, params: &Json) -> EngineResult<Json> {
    let table = str_param(command, params, "table")?;
    let limit = params.get("limit").and_then(Json::as_u64).unwrap_or(20).min(1_000);
    let rows = source.preview(table, usize::try_from(limit).unwrap_or(1_000))?;
    Ok(json!({"columns": rows.columns, "rows": rows.rows}))
}

/// The query with the report's parameters bound. Parameters are resolved only when the query
/// uses them, so a report with a required parameter can still load data that doesn't need it.
#[cfg(not(target_arch = "wasm32"))]
fn bound_query(s: &Session, command: &str, query: &str, params: &Json, dialect: reportcraft_data::Dialect) -> EngineResult<reportcraft_data::Bound> {
    let declared: Vec<&str> = s.report.parameters.iter().map(|p| p.name.as_str()).collect();
    let probe = reportcraft_data::bind(query, &std::collections::BTreeMap::new(), &declared, dialect)?;
    if probe.values.is_empty() {
        return Ok(probe);
    }
    let resolved = resolve_parameters(s, command, params)?;
    Ok(reportcraft_data::bind(query, &resolved, &declared, dialect)?)
}

/// A query being run, recorded in the log when it ends.
#[cfg(not(target_arch = "wasm32"))]
struct Logged {
    entry: crate::QueryLogEntry,
    started: std::time::Instant,
}

#[cfg(not(target_arch = "wasm32"))]
impl Logged {
    fn new(command: &str, source: &str, target: String, bound: &reportcraft_data::Bound) -> Self {
        let entry = crate::QueryLogEntry {
            at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            command: command.to_owned(),
            source: source.to_owned(),
            target,
            sql: bound.sql.clone(),
            values: bound.values.iter().map(reportcraft_data::BindValue::as_text).collect(),
            duration_ms: 0,
            rows: None,
            error: None,
            from_cache: false,
        };
        // Native only (this type doesn't exist in the web build, which has no clock).
        #[allow(clippy::disallowed_methods)]
        let started = std::time::Instant::now();
        Self { entry, started }
    }

    fn finish(mut self, log: &mut crate::QueryLog, result: reportcraft_core::Result<RowSet>) -> reportcraft_core::Result<RowSet> {
        self.entry.duration_ms = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        match &result {
            Ok(rows) => self.entry.rows = Some(rows.rows.len()),
            Err(e) => self.entry.error = Some(e.to_string()),
        }
        log.push(self.entry);
        result
    }
}

/// The memory the loaded rows may take (`maxMegabytes`).
#[cfg(not(target_arch = "wasm32"))]
fn max_bytes_of(params: &Json) -> usize {
    params
        .get("maxMegabytes")
        .and_then(Json::as_u64)
        .and_then(|mb| usize::try_from(mb).ok())
        .map_or(reportcraft_core::limits::DEFAULT_DATA_BYTES, |mb| mb.max(1).saturating_mul(1 << 20))
}

/// Optional row limit and time limit shared by the database loaders.
#[cfg(not(target_arch = "wasm32"))]
fn limits_of(params: &Json) -> (Option<usize>, std::time::Duration) {
    let limit = params.get("limit").and_then(Json::as_u64).map(|n| usize::try_from(n).unwrap_or(usize::MAX));
    let timeout = params.get("timeoutSeconds").and_then(Json::as_u64).unwrap_or(30).clamp(1, 3_600);
    (limit, std::time::Duration::from_secs(timeout))
}

#[cfg(not(target_arch = "wasm32"))]
fn data_load_postgres(s: &mut Session, params: &Json) -> EngineResult<Json> {
    const CMD: &str = "data.loadPostgres";
    let connection = str_param(CMD, params, "connection")?;
    let query = str_param(CMD, params, "query")?;
    let (limit, timeout) = limits_of(params);
    let bound = bound_query(s, CMD, query, params, reportcraft_data::Dialect::Postgres)?;
    let source = reportcraft_postgres::PostgresSource::new(connection, bound.sql.clone())
        .with_binds(bound.values.clone())
        .with_timeout(timeout)
        .with_cancel(s.cancel().clone())
        .with_max_bytes(max_bytes_of(params));
    let target = source.target();
    load_from_database(s, Query { command: CMD, source: "postgres", target, bound: &bound, limit }, params, || source.read(limit))
}

#[cfg(not(target_arch = "wasm32"))]
fn data_describe_postgres(_: &mut Session, params: &Json) -> EngineResult<Json> {
    let connection = str_param("data.describePostgres", params, "connection")?;
    describe(&reportcraft_postgres::PostgresSource::database(connection), params)
}

#[cfg(not(target_arch = "wasm32"))]
fn data_preview_postgres(_: &mut Session, params: &Json) -> EngineResult<Json> {
    const CMD: &str = "data.previewPostgres";
    let connection = str_param(CMD, params, "connection")?;
    preview(&reportcraft_postgres::PostgresSource::database(connection), CMD, params)
}

#[cfg(target_arch = "wasm32")]
fn data_load_postgres(_: &mut Session, _: &Json) -> EngineResult<Json> {
    Err(reportcraft_core::Error::Unsupported("PostgreSQL is not available in the web build".into()).into())
}

#[cfg(target_arch = "wasm32")]
fn data_describe_postgres(_: &mut Session, _: &Json) -> EngineResult<Json> {
    Err(reportcraft_core::Error::Unsupported("PostgreSQL is not available in the web build".into()).into())
}

#[cfg(target_arch = "wasm32")]
fn data_preview_postgres(_: &mut Session, _: &Json) -> EngineResult<Json> {
    Err(reportcraft_core::Error::Unsupported("PostgreSQL is not available in the web build".into()).into())
}

#[cfg(target_arch = "wasm32")]
fn data_describe_sqlite(_: &mut Session, _: &Json) -> EngineResult<Json> {
    Err(reportcraft_core::Error::Unsupported("SQLite is not available in the web build".into()).into())
}

#[cfg(target_arch = "wasm32")]
fn data_preview_sqlite(_: &mut Session, _: &Json) -> EngineResult<Json> {
    Err(reportcraft_core::Error::Unsupported("SQLite is not available in the web build".into()).into())
}

#[cfg(target_arch = "wasm32")]
fn data_load_sqlite(_: &mut Session, _: &Json) -> EngineResult<Json> {
    Err(reportcraft_core::Error::Unsupported("SQLite is not available in the web build".into()).into())
}

fn data_preview(s: &mut Session, params: &Json) -> EngineResult<Json> {
    let limit = params.get("limit").and_then(Json::as_u64).unwrap_or(20).min(1_000);
    let limit = usize::try_from(limit).unwrap_or(1_000);
    let rows: Vec<_> = s.rows.rows.iter().take(limit).collect();
    Ok(json!({"columns": s.rows.columns, "rows": rows, "total": s.rows.rows.len()}))
}

fn data_query_log(s: &mut Session, params: &Json) -> EngineResult<Json> {
    let limit = params.get("limit").and_then(Json::as_u64).unwrap_or(50);
    let limit = usize::try_from(limit).unwrap_or(usize::MAX).min(crate::query_log::MAX_ENTRIES);
    let entries: Vec<_> = s.query_log.last(limit).collect();
    Ok(json!({"entries": entries, "kept": s.query_log.len(), "dropped": s.query_log.dropped()}))
}

fn data_clear_cache(s: &mut Session, _: &Json) -> EngineResult<Json> {
    let cleared = s.cache.len();
    s.cache.clear();
    Ok(json!({"cleared": cleared}))
}

fn data_clear_query_log(s: &mut Session, _: &Json) -> EngineResult<Json> {
    let cleared = s.query_log.len();
    s.query_log.clear();
    Ok(json!({"cleared": cleared}))
}

fn report_validate(s: &mut Session, params: &Json) -> EngineResult<Json> {
    use reportcraft_model::{Diagnostic, Severity};
    let columns = (!s.rows.columns.is_empty()).then_some(s.rows.columns.as_slice());
    let datasets: std::collections::BTreeMap<String, Vec<reportcraft_core::Column>> =
        s.datasets.iter().map(|(name, rows)| (name.clone(), rows.columns.clone())).collect();
    let diagnostics = match params.get("json").and_then(Json::as_str) {
        Some(text) => match Report::from_json(text) {
            Ok(report) => report.check_with_datasets(columns, &datasets),
            Err(e) => vec![Diagnostic { severity: Severity::Error, location: "file".into(), message: e.to_string() }],
        },
        None => s.report.check_with_datasets(columns, &datasets),
    };
    let errors = diagnostics.iter().filter(|d| d.severity == Severity::Error).count();
    Ok(json!({
        "valid": errors == 0,
        "errors": errors,
        "warnings": diagnostics.len() - errors,
        "dataChecked": columns.is_some(),
        "diagnostics": diagnostics,
    }))
}

fn report_parameters(s: &mut Session, _: &Json) -> EngineResult<Json> {
    serde_json::to_value(&s.report.parameters).map(|p| json!({"parameters": p})).map_err(|e| EngineError::Internal(e.to_string()))
}

/// The `params` object of a command as supplied values: a string, number or boolean is one value,
/// an array several, `null` none.
fn supplied_values(params: &Json) -> std::collections::BTreeMap<String, Vec<String>> {
    let one = |v: &Json| match v {
        Json::Null => None,
        Json::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    };
    params
        .get("params")
        .and_then(Json::as_object)
        .map(|map| {
            map.iter()
                .map(|(k, v)| {
                    let values = match v {
                        Json::Array(items) => items.iter().take(reportcraft_model::MAX_VALUES + 1).filter_map(one).collect(),
                        other => one(other).into_iter().collect(),
                    };
                    (k.clone(), values)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The report's parameters resolved for one run, or the problems as a parameter error.
pub(crate) fn resolve_parameters(
    s: &Session,
    command: &str,
    params: &Json,
) -> EngineResult<std::collections::BTreeMap<String, reportcraft_model::ParamValue>> {
    reportcraft_model::resolve_parameters(&s.report.parameters, &supplied_values(params))
        .map_err(|problems| bad(command, problems.iter().map(|d| d.message.as_str()).collect::<Vec<_>>().join("; ")))
}

/// The session's cancel flag, with the render's time limit (`timeoutSeconds`) on native builds.
#[cfg(not(target_arch = "wasm32"))]
fn render_limit(s: &Session, params: &Json) -> reportcraft_core::Cancel {
    let seconds = params.get("timeoutSeconds").and_then(Json::as_u64).unwrap_or(120).clamp(1, 3_600);
    s.cancel().with_time_limit(std::time::Duration::from_secs(seconds))
}

#[cfg(target_arch = "wasm32")]
fn render_limit(s: &Session, _: &Json) -> reportcraft_core::Cancel {
    s.cancel().clone()
}

fn report_render(s: &mut Session, params: &Json) -> EngineResult<Json> {
    let resolved = resolve_parameters(s, "report.render", params)?;
    let mut values: Params = resolved.iter().map(|(k, v)| (k.clone(), v.display())).collect();
    // Parameter names are letters, digits and `_`, so these can't clash with one.
    if let Some(at) = &s.refreshed_at {
        values.insert(reportcraft_layout::REFRESHED_AT.to_owned(), at.clone());
    }
    if let Some(now) = now_local() {
        values.insert(reportcraft_layout::PRINTED_AT.to_owned(), now);
    }
    let doc = reportcraft_layout::render(&s.report, &s.rows, &s.datasets, &values, &render_limit(s, params))?;
    match params.get("format").and_then(Json::as_str).unwrap_or("html") {
        "html" => Ok(json!({"pages": doc.pages.len(), "html": to_html(&doc), "refreshedAt": s.refreshed_at})),
        "json" => serde_json::to_value(&doc).map_err(|e| EngineError::Internal(e.to_string())),
        other => Err(bad("report.render", format!("unknown format `{other}` (html or json)"))),
    }
}
