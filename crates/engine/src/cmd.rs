//! The command registry. A command is an id, a label, its menu path, a parameter description
//! and `run`. Ids follow the menus (`file.open`, `data.loadCsv`, `report.render`) plus primitives.

use reportcraft_data::{CsvSource, DataSource};
use reportcraft_layout::{Params, layout, to_html};
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
    CommandSpec { id: "data.loadCsv", label: "Load CSV data", menu: "Data", params: r#"{"csv": string (header row first)}"#, run: data_load_csv },
    CommandSpec {
        id: "data.loadSqlite",
        label: "Load SQLite data",
        menu: "Data",
        params: r#"{"path": string (database file, opened read-only), "query": string (one SELECT), "limit"?: integer, "timeoutSeconds"?: integer (default 30, at most 3600)}"#,
        run: data_load_sqlite,
    },
    CommandSpec {
        id: "data.preview",
        label: "Preview data",
        menu: "Data",
        params: r#"{"limit"?: integer (default 20, at most 1000)}"#,
        run: data_preview,
    },
    CommandSpec {
        id: "report.render",
        label: "Render",
        menu: "Report",
        params: r#"{"format"?: "html" | "json" (default "html"), "params"?: {name: string}}"#,
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
    *s = Session { report: Report::new(title), rows: Default::default() };
    Ok(json!({"title": title}))
}

fn file_open_sample(s: &mut Session, params: &Json) -> EngineResult<Json> {
    let name = params.get("name").and_then(Json::as_str).unwrap_or("rentals");
    let (report, csv) = sample::by_name(name).ok_or_else(|| bad("file.openSample", format!("no sample named `{name}`")))?;
    s.rows = CsvSource::new(csv).read(None)?;
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
    s.rows = CsvSource::new(str_param("data.loadCsv", params, "csv")?).read(None)?;
    Ok(json!({"columns": s.rows.columns, "rows": s.rows.rows.len()}))
}

#[cfg(not(target_arch = "wasm32"))]
fn data_load_sqlite(s: &mut Session, params: &Json) -> EngineResult<Json> {
    const CMD: &str = "data.loadSqlite";
    let path = str_param(CMD, params, "path")?;
    let query = str_param(CMD, params, "query")?;
    let limit = params.get("limit").and_then(Json::as_u64).map(|n| usize::try_from(n).unwrap_or(usize::MAX));
    let timeout = params.get("timeoutSeconds").and_then(Json::as_u64).unwrap_or(30).clamp(1, 3_600);
    let source = reportcraft_sqlite::SqliteSource::new(path, query).with_timeout(std::time::Duration::from_secs(timeout));
    s.rows = source.read(limit)?;
    Ok(json!({"columns": s.rows.columns, "rows": s.rows.rows.len()}))
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

fn report_render(s: &mut Session, params: &Json) -> EngineResult<Json> {
    let mut values = Params::new();
    if let Some(map) = params.get("params").and_then(Json::as_object) {
        for (k, v) in map {
            let text = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
            values.insert(k.clone(), text);
        }
    }
    let doc = layout(&s.report, &s.rows, &values)?;
    match params.get("format").and_then(Json::as_str).unwrap_or("html") {
        "html" => Ok(json!({"pages": doc.pages.len(), "html": to_html(&doc)})),
        "json" => serde_json::to_value(&doc).map_err(|e| EngineError::Internal(e.to_string())),
        other => Err(bad("report.render", format!("unknown format `{other}` (html or json)"))),
    }
}
