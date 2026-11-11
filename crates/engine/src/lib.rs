//! ReportCraft engine (L5): the session and the command registry.
//!
//! Everything is a command: the CLI, the MCP server and (later) the egui interface and the control
//! channel all call `Session::execute(id, params)`. Programmatic calls never open dialogs.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

// The web build reads no database, so it never fills the cache.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub mod base64;
mod cache;
mod cmd;
pub mod query_log;
pub mod sample;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use reportcraft_core::{Cancel, Error, RowSet};
use reportcraft_model::Report;
use serde_json::Value as Json;

pub use cache::QueryCache;
pub use cmd::{Access, CommandSpec, commands, find_command};

/// What a session may do beyond its own state. A person at the command line or in the app may do
/// everything; an MCP agent, by default, only what the user set up: saved connections, and no
/// external action without a person's confirmation (specification §11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// May open a database from a raw file path or connection string (else only saved ones).
    pub raw_sources: bool,
    /// May confirm external actions (print, save or delete a connection).
    pub external_actions: bool,
}

impl Policy {
    /// A person: everything.
    pub const PERSON: Policy = Policy { raw_sources: true, external_actions: true };
    /// An agent: saved connections only, external actions proposed but never confirmed.
    pub const AGENT: Policy = Policy { raw_sources: false, external_actions: false };
}

impl Default for Policy {
    fn default() -> Self {
        Policy::PERSON
    }
}
pub use query_log::{QueryLog, QueryLogEntry};

/// What a command can fail with: the core error, plus the engine's own cases.
#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum EngineError {
    #[error(transparent)]
    Core(#[from] Error),
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("bad parameters for `{command}`: {message}")]
    BadParams { command: String, message: String },
    #[error("{0} (the command was undone)")]
    Internal(String),
}

pub type EngineResult<T> = Result<T, EngineError>;

/// The open report, the rows it reads and the queries run so far.
#[derive(Debug, Clone)]
pub struct Session {
    pub report: Report,
    /// Shared, so copying the session (before each command, for the cache) doesn't copy rows.
    pub rows: Arc<RowSet>,
    /// When the rows were read (RFC 3339, local time); `None` in the web build.
    pub refreshed_at: Option<String>,
    /// Named datasets, read by subreports (loaded with `"dataset": name`).
    pub datasets: reportcraft_layout::Datasets,
    /// Results kept for loads that ask for them (`cacheSeconds`).
    pub cache: QueryCache,
    /// Kept when another report is opened: it belongs to the session, not the report.
    pub query_log: QueryLog,
    /// Raised through `cancel_handle` to stop the running command.
    cancel: Cancel,
    /// What the session may do beyond its own state (an MCP server narrows it).
    pub policy: Policy,
    /// Earlier report definitions, most recent last (`edit.undo`), and the ones undone
    /// (`edit.redo`).
    pub(crate) undo: Vec<Report>,
    pub(crate) redo: Vec<Report>,
    /// Saved connections, opened on first use (the user's settings and keychain unless a test
    /// gave others). Shared by clones, so a restored session keeps them.
    #[cfg(not(target_arch = "wasm32"))]
    connections: Arc<std::sync::Mutex<Option<reportcraft_connections::Connections>>>,
}

/// Report definitions kept for undo.
pub const MAX_UNDO: usize = 100;

impl Default for Session {
    fn default() -> Self {
        Self {
            report: Report::new("Untitled report"),
            rows: Arc::default(),
            refreshed_at: None,
            datasets: reportcraft_layout::Datasets::new(),
            cache: QueryCache::default(),
            query_log: QueryLog::default(),
            cancel: Cancel::default(),
            policy: Policy::PERSON,
            undo: Vec::new(),
            redo: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            connections: Arc::default(),
        }
    }
}

impl Session {
    /// Runs one command. A panic that escapes a command is caught and the session restored as it
    /// was before the command: a last-resort guard, not a licence to panic.
    pub fn execute(&mut self, id: &str, params: &Json) -> EngineResult<Json> {
        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_owned()))?;
        let before = self.clone();
        let result = match catch_unwind(AssertUnwindSafe(|| (spec.run)(self, params))) {
            Ok(result) => result,
            Err(_) => {
                *self = before.clone();
                log::error!("command `{id}` panicked; session restored");
                Err(EngineError::Internal(format!("command `{id}` failed unexpectedly")))
            }
        };
        // A stop request ends with the command it was meant for.
        self.cancel.reset();
        // Any command that changed the report can be undone (undo and redo keep their own lists).
        if result.is_ok() && !matches!(id, "edit.undo" | "edit.redo") && self.report != before.report {
            if self.undo.len() == MAX_UNDO {
                self.undo.remove(0);
            }
            self.undo.push(before.report);
            self.redo.clear();
        }
        result
    }

    /// How many edits can be undone and redone.
    pub fn history(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }

    /// A handle another thread uses to stop the command running now (`Cancel::cancel`). The
    /// command fails with a `cancelled` error and, like any failed command, leaves the session as
    /// it was. The request is dropped when that command ends; raising it while no command runs
    /// stops the next one, so callers raise it only while a command is running.
    /// A session that uses these saved connections instead of the user's (tests, embedding).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_saved_connections(connections: reportcraft_connections::Connections) -> Self {
        Self { connections: Arc::new(std::sync::Mutex::new(Some(connections))), ..Self::default() }
    }

    /// Runs `f` on the saved connections, opening the user's on first use.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn with_connections<T>(
        &self,
        f: impl FnOnce(&mut reportcraft_connections::Connections) -> reportcraft_core::Result<T>,
    ) -> reportcraft_core::Result<T> {
        let mut guard = self.connections.lock().map_err(|_| Error::Io("saved connections are unavailable".into()))?;
        // The engine's own tests never touch the user's settings or keychain.
        let open = if cfg!(test) { reportcraft_connections::Connections::in_memory } else { reportcraft_connections::Connections::open_user };
        f(guard.get_or_insert_with(open))
    }

    pub fn cancel_handle(&self) -> Cancel {
        self.cancel.clone()
    }

    /// The flag commands pass to long work.
    pub(crate) fn cancel(&self) -> &Cancel {
        &self.cancel
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_command_survives_empty_params() {
        for spec in commands() {
            let mut s = Session::default();
            let _ = s.execute(spec.id, &json!({}));
            let _ = s.execute(spec.id, &Json::Null);
        }
    }

    #[test]
    fn renders_pdf_as_base64_with_the_fonts_used() {
        if let Err(e) = reportcraft_pdf::Fonts::find() {
            assert!(std::env::var_os("CI").is_none(), "no font on CI: {e}");
            return;
        }
        let mut s = Session::default();
        s.execute("file.openSample", &json!({"name": "rentals"})).unwrap();
        let r = s.execute("report.render", &json!({"format": "pdf"})).unwrap();
        let bytes = base64::decode(r["pdfBase64"].as_str().unwrap()).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        assert_eq!(r["bytes"], json!(bytes.len()));
        assert_eq!(r["pages"], json!(1));
        assert!(r["fonts"]["regular"].as_str().is_some_and(|f| !f.is_empty()));
        let err = s.execute("report.render", &json!({"format": "rtf"})).unwrap_err().to_string();
        assert!(err.contains("html, pdf, xlsx, docx or json"), "{err}");
    }

    #[test]
    fn unknown_command_is_an_error() {
        assert!(matches!(Session::default().execute("nope", &json!({})), Err(EngineError::UnknownCommand(_))));
    }

    #[test]
    fn sqlite_rows_feed_the_sample_report() {
        let dir = std::env::temp_dir().join(format!("reportcraft-engine-sqlite-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("rentals.db");
        rusqlite::Connection::open(&db)
            .unwrap()
            .execute_batch(
                "CREATE TABLE r (branch TEXT, item TEXT, days INT, amount REAL);
                 INSERT INTO r VALUES ('Chambery', 'Tile cutter', 2, 64.0), ('Chambery', 'Pallet truck', 5, 110.0);",
            )
            .unwrap();
        let mut s = Session::default();
        s.execute("file.openSample", &json!({"name": "rentals"})).unwrap();
        let path = db.to_string_lossy();
        let loaded = s.execute("data.loadSqlite", &json!({"path": path, "query": "SELECT * FROM r"})).unwrap();
        assert_eq!(loaded["rows"], json!(2));
        let html = s.execute("report.render", &json!({})).unwrap()["html"].as_str().unwrap().to_owned();
        assert!(html.contains("Chambery") && html.contains("174"), "branch and its total are shown");
        let schema = s.execute("data.describeSqlite", &json!({"path": path})).unwrap();
        assert_eq!(schema["tables"][0]["name"], json!("r"));
        assert_eq!(schema["tables"][0]["columns"][2]["field_type"], json!("integer"));
        let table = s.execute("data.describeSqlite", &json!({"path": path, "table": "R"})).unwrap();
        assert_eq!(table["columns"].as_array().map(Vec::len), Some(4));
        let preview = s.execute("data.previewSqlite", &json!({"path": path, "table": "r", "limit": 1})).unwrap();
        assert_eq!(preview["rows"].as_array().map(Vec::len), Some(1));
        assert!(s.execute("data.describeSqlite", &json!({"path": path, "table": "missing"})).is_err());
        s.report.parameters.push(reportcraft_model::Parameter {
            kind: reportcraft_model::ParamType::Integer,
            required: true,
            ..reportcraft_model::Parameter::text("min_days", "", None)
        });
        let q = "SELECT * FROM r WHERE days >= :min_days";
        let bound = s.execute("data.loadSqlite", &json!({"path": path, "query": q, "params": {"min_days": 3}})).unwrap();
        assert_eq!(bound["rows"], json!(1), "only the 5-day rental");
        let missing = s.execute("data.loadSqlite", &json!({"path": path, "query": q}));
        assert!(matches!(missing, Err(EngineError::BadParams { .. })), "a required parameter the query uses");
        let typo = s.execute("data.loadSqlite", &json!({"path": path, "query": "SELECT * FROM r WHERE days >= :min_dayz"}));
        assert!(typo.unwrap_err().to_string().contains("doesn't declare"));
        s.execute("data.loadSqlite", &json!({"path": path, "query": "SELECT * FROM r"})).unwrap();
        let refused = s.execute("data.loadSqlite", &json!({"path": path, "query": "DELETE FROM r"}));
        assert!(refused.is_err());
        assert_eq!(s.rows.rows.len(), 2, "a refused command leaves the loaded rows as they were");

        let log = s.execute("data.queryLog", &json!({})).unwrap();
        let entries = log["entries"].as_array().unwrap();
        // Queries refused before running (missing parameter, undeclared name) never reach the database.
        assert_eq!(entries.len(), 4, "{log}");
        assert_eq!(entries[0]["rows"], json!(2));
        assert_eq!(entries[0]["source"], json!("sqlite"));
        assert_eq!(entries[0]["target"], json!(path));
        assert_eq!(entries[1]["values"], json!(["3"]), "bound values, in order");
        assert!(entries[1]["sql"].as_str().unwrap().contains("?1"), "the SQL as sent");
        assert!(entries[3]["error"].is_string() && entries[3].get("rows").is_none(), "a failed query is logged with its error");
        assert!(entries[3]["at"].as_str().unwrap().ends_with('Z'));
        assert_eq!(s.execute("data.queryLog", &json!({"limit": 1})).unwrap()["entries"].as_array().map(Vec::len), Some(1));
        s.execute("file.new", &json!({})).unwrap();
        assert_eq!(s.query_log.len(), 4, "the log belongs to the session, not the report");
        assert_eq!(s.execute("data.clearQueryLog", &json!({})).unwrap()["cleared"], json!(4));
        assert!(s.query_log.is_empty());
    }

    #[test]
    fn a_running_load_stops_on_request_and_the_session_carries_on() {
        let dir = std::env::temp_dir().join(format!("reportcraft-engine-cancel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("x.db");
        rusqlite::Connection::open(&db).unwrap().execute_batch("CREATE TABLE t (a INT); INSERT INTO t VALUES (1);").unwrap();
        let path = db.to_string_lossy();
        let mut s = Session::default();
        s.execute("file.openSample", &json!({"name": "rentals"})).unwrap();
        let handle = s.cancel_handle();
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            handle.cancel();
        });
        let endless = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) SELECT count(*) FROM n";
        let stopped = s.execute("data.loadSqlite", &json!({"path": path, "query": endless}));
        stopper.join().unwrap();
        assert!(matches!(&stopped, Err(EngineError::Core(Error::Cancelled(_)))), "{stopped:?}");
        assert_eq!(s.rows.rows.len(), 7, "the sample rows are still loaded");
        assert!(s.query_log.last(1).next().unwrap().error.as_deref().unwrap().contains("stopped on request"));
        let next = s.execute("data.loadSqlite", &json!({"path": path, "query": "SELECT * FROM t"})).unwrap();
        assert_eq!(next["rows"], json!(1), "the request ended with the command it stopped");
        let tight = s.execute("data.loadSqlite", &json!({"path": path, "query": "SELECT printf('%.2000000c', 'x')", "maxMegabytes": 1}));
        assert!(tight.unwrap_err().to_string().contains("1 MiB"));
    }

    #[test]
    fn loads_reuse_cached_rows_only_when_asked() {
        let dir = std::env::temp_dir().join(format!("reportcraft-engine-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("x.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE t (a INT); INSERT INTO t VALUES (1);").unwrap();
        let path = db.to_string_lossy();
        let mut s = Session::default();
        let load = |s: &mut Session, cache: u64| {
            s.execute("data.loadSqlite", &json!({"path": path, "query": "SELECT * FROM t", "cacheSeconds": cache})).unwrap()
        };
        let first = load(&mut s, 60);
        assert_eq!((first["rows"].clone(), first["fromCache"].clone()), (json!(1), json!(false)));
        assert!(first["refreshedAt"].is_string());
        conn.execute_batch("INSERT INTO t VALUES (2);").unwrap();
        let again = load(&mut s, 60);
        assert_eq!((again["rows"].clone(), again["fromCache"].clone()), (json!(1), json!(true)), "the cached rows, not the new one");
        assert_eq!(again["refreshedAt"], first["refreshedAt"], "and when they were read");
        let log = s.execute("data.queryLog", &json!({})).unwrap();
        assert_eq!(log["entries"][1]["fromCache"], json!(true));
        assert_eq!(log["entries"][0].get("fromCache"), None);
        assert_eq!(load(&mut s, 0)["rows"], json!(2), "no cacheSeconds: the database is read");
        let html = s.execute("report.render", &json!({}));
        assert!(html.unwrap()["refreshedAt"].is_string());
        assert_eq!(s.execute("data.clearCache", &json!({})).unwrap()["cleared"], json!(1));
        assert_eq!(load(&mut s, 60)["fromCache"], json!(false));
    }

    #[test]
    fn subreports_read_named_datasets() {
        let mut s = Session::default();
        let sub = json!({
            "x_mm": 0, "y_mm": 6, "width_mm": 100, "height_mm": 5,
            "content": {"kind": "subreport", "dataset": "lines", "links": [{"parent": "id", "child": "order_id"}],
                "report": {"format_version": 1, "title": "Lines",
                    "detail": {"height_mm": 5, "elements": [{"x_mm": 0, "y_mm": 0, "width_mm": 60, "height_mm": 5, "content": {"kind": "field", "field": "item"}}]}}}
        });
        let report = json!({"format_version": 1, "title": "Orders",
            "detail": {"height_mm": 12, "elements": [{"x_mm": 0, "y_mm": 0, "width_mm": 60, "height_mm": 5, "content": {"kind": "field", "field": "id"}}, sub]}});
        s.execute("report.load", &json!({"json": report.to_string()})).unwrap();
        s.execute("data.loadCsv", &json!({"csv": "id\n7\n8\n"})).unwrap();
        let missing = s.execute("report.validate", &json!({})).unwrap();
        assert!(missing["diagnostics"].to_string().contains("dataset `lines` is not loaded"), "{missing}");
        assert!(s.execute("report.render", &json!({})).unwrap_err().to_string().contains("lines"));
        let loaded = s.execute("data.loadCsv", &json!({"csv": "order_id,item\n7,Tent\n7,Stove\n9,Lamp\n", "dataset": "lines"})).unwrap();
        assert_eq!((loaded["dataset"].clone(), loaded["rows"].clone()), (json!("lines"), json!(3)));
        assert_eq!(s.rows.rows.len(), 2, "the report's own data is untouched");
        assert_eq!(s.execute("report.validate", &json!({})).unwrap()["errors"], json!(0));
        let html = s.execute("report.render", &json!({})).unwrap()["html"].as_str().unwrap().to_owned();
        assert!(html.contains("Tent") && html.contains("Stove") && !html.contains("Lamp"), "only order 7's lines");
        assert_eq!(s.execute("data.datasets", &json!({})).unwrap()["datasets"][0]["rows"], json!(3));
        assert!(s.execute("data.loadCsv", &json!({"csv": "a\n1\n", "dataset": "bad name"})).is_err());
        s.execute("file.new", &json!({})).unwrap();
        assert!(s.datasets.is_empty());
    }

    #[test]
    fn validate_checks_the_open_report_against_its_data() {
        let mut s = Session::default();
        s.execute("file.openSample", &json!({"name": "rentals"})).unwrap();
        let ok = s.execute("report.validate", &json!({})).unwrap();
        assert_eq!((ok["valid"].clone(), ok["errors"].clone(), ok["dataChecked"].clone()), (json!(true), json!(0), json!(true)));
        s.execute("data.loadCsv", &json!({"csv": "region,total\nNorth,3\n"})).unwrap();
        let bad = s.execute("report.validate", &json!({})).unwrap();
        assert_eq!(bad["valid"], json!(false));
        assert!(bad["diagnostics"].as_array().unwrap().iter().any(|d| d["location"] == json!("groups[0]")));
        let broken = s.execute("report.validate", &json!({"json": "{not json"})).unwrap();
        assert_eq!(broken["diagnostics"][0]["location"], json!("file"));
    }

    #[test]
    fn render_checks_typed_parameters() {
        let mut s = Session::default();
        s.execute("file.openSample", &json!({"name": "rentals"})).unwrap();
        s.report.parameters.push(reportcraft_model::Parameter {
            kind: reportcraft_model::ParamType::Integer,
            required: true,
            ..reportcraft_model::Parameter::text("min_days", "Minimum days", None)
        });
        let listed = s.execute("report.parameters", &json!({})).unwrap();
        assert_eq!(listed["parameters"][1]["type"], json!("integer"));
        let missing = s.execute("report.render", &json!({}));
        assert!(matches!(missing, Err(EngineError::BadParams { .. })));
        let wrong = s.execute("report.render", &json!({"params": {"min_days": "three"}})).unwrap_err().to_string();
        assert!(wrong.contains("`three` is not a whole number"), "{wrong}");
        assert!(s.execute("report.render", &json!({"params": {"min_days": 3}})).is_ok(), "a JSON number is accepted");
        assert!(s.execute("report.render", &json!({"params": {"min_days": [3, 4]}})).is_err(), "one value only");
    }

    #[test]
    fn sample_renders_to_html() {
        let mut s = Session::default();
        s.execute("file.openSample", &json!({"name": "rentals"})).unwrap();
        let out = s.execute("report.render", &json!({"format": "html"})).unwrap();
        let html = out.get("html").and_then(Json::as_str).unwrap();
        assert!(html.contains("Lyon") && html.contains("1 / 1"));
    }

    #[test]
    fn formulas_are_tried_checked_and_rendered() {
        let mut s = Session::default();
        s.execute("file.openSample", &json!({"name": "rentals"})).unwrap();
        s.report.formulas.push(reportcraft_model::FormulaField { name: "daily".into(), expression: "ROUND([amount] / [days], 2)".into() });
        let tried = s.execute("report.evaluateFormula", &json!({"expression": "[daily] & \" / day\"", "limit": 2})).unwrap();
        assert_eq!(tried["values"], json!(["45 / day", "230.17 / day"]));
        assert_eq!(tried["rows"], json!(7));
        let share = s.execute("report.evaluateFormula", &json!({"expression": "ROUND(SUM([daily]) / COUNT([daily]), 2)", "limit": 1})).unwrap();
        assert_eq!(share["values"], json!([87.77]));
        assert!(s.execute("report.evaluateFormula", &json!({"expression": "SUM("})).unwrap_err().to_string().contains("formula"));
        assert!(matches!(s.execute("report.evaluateFormula", &json!({})), Err(EngineError::BadParams { .. })));

        let formula = |e: &str| reportcraft_model::Element {
            x_mm: 120.0,
            y_mm: 0.5,
            width_mm: 30.0,
            height_mm: 5.0,
            style: Default::default(),
            content: reportcraft_model::Content::Formula { expression: e.into() },
            format: None,
            visible_if: None,
            background_formula: None,
            color_formula: None,
        };
        s.report.detail.elements.push(formula("[daily] & \" €/d\""));
        let html = s.execute("report.render", &json!({})).unwrap()["html"].as_str().unwrap().to_owned();
        assert!(html.contains("230.17 €/d"), "{html}");
        s.report.detail.elements.push(formula("[dailly]"));
        let checked = s.execute("report.validate", &json!({})).unwrap();
        assert_eq!(checked["valid"], json!(false));
        assert!(checked["diagnostics"].to_string().contains("unknown field `dailly`"), "{checked}");
    }

    #[test]
    fn renders_xlsx_and_docx_as_base64() {
        let mut s = Session::default();
        s.execute("file.openSample", &json!({"name": "rentals"})).unwrap();
        for (format, key) in [("xlsx", "xlsxBase64"), ("docx", "docxBase64")] {
            let r = s.execute("report.render", &json!({"format": format})).unwrap();
            let bytes = base64::decode(r[key].as_str().unwrap()).unwrap();
            assert!(bytes.starts_with(b"PK"), "{format} is a zip package");
            assert_eq!(r["bytes"], json!(bytes.len()));
            assert_eq!(r["pages"], json!(1));
        }
        let unknown = s.execute("report.render", &json!({"format": "odt"})).unwrap_err().to_string();
        assert!(unknown.contains("xlsx, docx"), "{unknown}");
    }

    #[test]
    fn print_says_what_it_would_send_until_confirmed() {
        let mut s = Session::default();
        s.execute("file.openSample", &json!({"name": "rentals"})).unwrap();
        for (params, wrong) in [
            (json!({"copies": 0}), "copies"),
            (json!({"copies": 100}), "copies"),
            (json!({"duplex": "both"}), "duplex"),
            (json!({"printer": ""}), "printer"),
        ] {
            let err = s.execute("file.print", &params).unwrap_err().to_string();
            assert!(err.contains(wrong), "{err}");
        }
        if let Err(e) = reportcraft_pdf::Fonts::find() {
            assert!(std::env::var_os("CI").is_none(), "no font on CI: {e}");
            return;
        }
        let r = s.execute("file.print", &json!({"printer": "Office", "copies": 2, "duplex": "long", "grayscale": true, "pages": "1"})).unwrap();
        assert_eq!(r["submitted"], json!(false), "no `confirm`: nothing is printed");
        assert_eq!((r["pages"].clone(), r["of"].clone(), r["copies"].clone()), (json!(1), json!(1), json!(2)));
        #[cfg(unix)]
        {
            let command = r["command"].to_string();
            assert!(command.contains("\"lp\"") && command.contains("\"Office\"") && command.contains("two-sided-long-edge"), "{command}");
        }
        assert!(s.execute("file.print", &json!({"pages": "2-"})).unwrap_err().to_string().contains("none of the 1"));
        assert!(s.execute("file.printers", &json!({})).unwrap()["printers"].is_array());
    }

    #[test]
    fn saved_connections_are_used_by_name_and_keep_passwords_out_of_results() {
        let dir = std::env::temp_dir().join(format!("reportcraft-saved-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("rentals.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE rentals (branch TEXT, amount REAL); INSERT INTO rentals VALUES ('Lyon', 10), ('Paris', 5);").unwrap();
        drop(conn);
        let mut s = Session::with_saved_connections(reportcraft_connections::Connections::in_memory());
        let save = json!({"name": "local", "kind": "sqlite", "path": db.display().to_string()});
        let proposed = s.execute("data.saveConnection", &save).unwrap();
        assert_eq!(proposed["saved"], json!(false), "nothing changes without confirm");
        assert_eq!(s.execute("data.connections", &json!({})).unwrap()["connections"], json!([]));
        let mut confirmed = save.clone();
        confirmed["confirm"] = json!(true);
        assert_eq!(s.execute("data.saveConnection", &confirmed).unwrap()["saved"], json!(true));
        let pg = json!({"name": "odoo", "kind": "postgres", "host": "127.0.0.1", "port": 1, "user": "rc", "database": "x", "password": "hunter2", "confirm": true});
        s.execute("data.saveConnection", &pg).unwrap();
        let listed = s.execute("data.connections", &json!({})).unwrap();
        assert_eq!(listed["connections"][1]["target"], json!("rc@127.0.0.1:1/x"));
        assert_eq!(listed["connections"][1]["password"], json!(true));
        assert!(!listed.to_string().contains("hunter2"), "{listed}");

        let loaded = s.execute("data.loadSqlite", &json!({"saved": "local", "query": "SELECT * FROM rentals ORDER BY branch"})).unwrap();
        assert_eq!(loaded["rows"], json!(2));
        assert_eq!(s.execute("data.describeSqlite", &json!({"saved": "local"})).unwrap()["tables"][0]["name"], json!("rentals"));
        assert_eq!(s.execute("data.testConnection", &json!({"name": "local"})).unwrap()["tables"], json!(1));
        let wrong = s.execute("data.loadSqlite", &json!({"saved": "odoo", "query": "SELECT 1"})).unwrap_err().to_string();
        assert!(wrong.contains("not SQLite"), "{wrong}");
        // Nothing listens on port 1: the error and the query log name the server, never the password.
        let failed = s.execute("data.loadPostgres", &json!({"saved": "odoo", "query": "SELECT 1", "timeoutSeconds": 5})).unwrap_err().to_string();
        let log = s.execute("data.queryLog", &json!({})).unwrap().to_string();
        assert!(!failed.contains("hunter2") && !log.contains("hunter2"), "{failed} {log}");
        assert!(log.contains("rc@127.0.0.1:1/x"), "{log}");

        assert_eq!(s.execute("data.deleteConnection", &json!({"name": "odoo"})).unwrap()["deleted"], json!(false));
        assert_eq!(s.execute("data.deleteConnection", &json!({"name": "odoo", "confirm": true})).unwrap()["deleted"], json!(true));
        assert!(s.execute("data.loadPostgres", &json!({"saved": "odoo", "query": "SELECT 1"})).unwrap_err().to_string().contains("saved: local"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_agent_session_uses_saved_connections_and_cannot_confirm_external_actions() {
        let mut s = Session::with_saved_connections(reportcraft_connections::Connections::in_memory());
        s.execute("data.saveConnection", &json!({"name": "local", "kind": "sqlite", "path": "/nonexistent/x.db", "confirm": true})).unwrap();
        s.policy = Policy::AGENT;
        for (id, params) in [
            ("data.loadSqlite", json!({"path": "/etc/passwd", "query": "SELECT 1"})),
            ("data.describeSqlite", json!({"path": "/etc/passwd"})),
            ("data.previewPostgres", json!({"connection": "host=evil", "table": "t"})),
            ("data.loadPostgres", json!({"connection": "host=evil", "query": "SELECT 1"})),
        ] {
            let err = s.execute(id, &params).unwrap_err().to_string();
            assert!(err.contains("saved connections only"), "{id}: {err}");
        }
        // A saved connection is fine (this one points nowhere, so the error is the file's).
        let err = s.execute("data.describeSqlite", &json!({"saved": "local"})).unwrap_err().to_string();
        assert!(!err.contains("saved connections only"), "{err}");
        // Proposing is fine; confirming is for a person.
        let proposed = json!({"name": "new", "kind": "sqlite", "path": "/tmp/y.db"});
        assert_eq!(s.execute("data.saveConnection", &proposed).unwrap()["saved"], json!(false));
        let mut confirmed = proposed.clone();
        confirmed["confirm"] = json!(true);
        assert!(s.execute("data.saveConnection", &confirmed).unwrap_err().to_string().contains("only a person"));
        assert!(s.execute("data.deleteConnection", &json!({"name": "local", "confirm": true})).unwrap_err().to_string().contains("only a person"));
        assert!(s.execute("file.print", &json!({"confirm": true})).unwrap_err().to_string().contains("only a person"));
        assert_eq!(s.execute("data.connections", &json!({})).unwrap()["connections"].as_array().map(Vec::len), Some(1));
        // Every command says what it touches.
        let external: Vec<&str> = commands().iter().filter(|c| c.access == Access::External).map(|c| c.id).collect();
        assert_eq!(external, ["file.print", "data.saveConnection", "data.deleteConnection"]);
    }

    #[test]
    fn reports_are_edited_by_commands_and_stay_valid() {
        let mut s = Session::default();
        s.execute("file.openSample", &json!({"name": "rentals"})).unwrap();
        let field =
            json!({"x_mm": 120, "y_mm": 0.5, "width_mm": 30, "height_mm": 5, "content": {"kind": "formula", "expression": "[amount] / [days]"}});
        let added = s.execute("report.addElement", &json!({"band": "detail", "element": field})).unwrap();
        let index = added["index"].as_u64().unwrap();
        assert!(matches!(s.report.detail.elements.last().map(|e| &e.content), Some(reportcraft_model::Content::Formula { .. })));
        let mut moved = field.clone();
        moved["x_mm"] = json!(100);
        s.execute("report.setElement", &json!({"band": "detail", "index": index, "element": moved})).unwrap();
        assert_eq!(s.report.detail.elements.last().map(|e| e.x_mm), Some(100.0));
        // Into the group footer: it goes last there.
        let r = s.execute("report.setElement", &json!({"band": "detail", "index": index, "element": moved, "move_to": "groups[0].footer"})).unwrap();
        assert_eq!(r["band"], json!("groups[0].footer"));
        let footer_len = s.report.groups[0].footer.elements.len();
        s.execute("report.removeElement", &json!({"band": "groups[0].footer", "index": footer_len - 1})).unwrap();
        assert_eq!(s.report.groups[0].footer.elements.len(), footer_len - 1);
        s.execute("report.setBand", &json!({"band": "detail", "height_mm": 7, "visible_if": "[days] > 2", "keep_with_next": true})).unwrap();
        assert_eq!((s.report.detail.height_mm, s.report.detail.visible_if.as_deref()), (7.0, Some("[days] > 2")));
        s.execute("report.setBand", &json!({"band": "detail", "visible_if": ""})).unwrap();
        assert!(s.report.detail.visible_if.is_none(), "an empty condition removes it");
        s.execute("report.setProperties", &json!({"title": "Rentals", "locale": "fr", "page": {"orientation": "landscape"}})).unwrap();
        assert_eq!((s.report.title.as_str(), s.report.locale.as_str(), s.report.page.size().0), ("Rentals", "fr", 297.0));
        // An edit that would break the report changes nothing.
        let before = s.report.clone();
        for (id, params) in [
            ("report.setBand", json!({"band": "detail", "height_mm": 5000})),
            (
                "report.addElement",
                json!({"band": "detail", "element": {"x_mm": -1, "y_mm": 0, "width_mm": 1, "height_mm": 1, "content": {"kind": "text", "text": "x"}}}),
            ),
            (
                "report.addElement",
                json!({"band": "detail", "element": {"x_mm": 0, "y_mm": 0, "width_mm": 1, "height_mm": 1, "content": {"kind": "formula", "expression": "1 +"}}}),
            ),
            ("report.removeElement", json!({"band": "detail", "index": 99})),
            ("report.setElement", json!({"band": "nowhere", "index": 0, "element": field})),
            ("report.setProperties", json!({"page": {"margin_mm": 500}})),
        ] {
            assert!(s.execute(id, &params).is_err(), "{id} {params}");
            assert_eq!(s.report, before, "{id} left the report as it was");
        }
    }

    #[test]
    fn report_edits_are_undone_and_redone() {
        let mut s = Session::default();
        s.execute("file.openSample", &json!({"name": "rentals"})).unwrap();
        let sample = s.report.clone();
        s.execute("report.setProperties", &json!({"title": "One"})).unwrap();
        s.execute("report.setProperties", &json!({"title": "Two"})).unwrap();
        // Read-only commands, loads that leave the report alone and refused edits aren't history.
        s.execute("report.validate", &json!({})).unwrap();
        s.execute("data.loadCsv", &json!({"csv": "branch,amount\nLyon,1\n"})).unwrap();
        let _ = s.execute("report.setBand", &json!({"band": "detail", "height_mm": 9000}));
        assert_eq!(s.history(), (3, 0), "the sample, One, Two");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.report.title, "One");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.report.title, "Rentals by branch");
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(s.report.title, "One");
        // A new edit forgets what was undone.
        s.execute("report.setProperties", &json!({"title": "Three"})).unwrap();
        assert!(s.execute("edit.redo", &json!({})).unwrap_err().to_string().contains("nothing to redo"));
        for _ in 0..3 {
            s.execute("edit.undo", &json!({})).unwrap();
        }
        assert_ne!(s.report, sample, "back before the sample: the untitled report");
        assert!(s.execute("edit.undo", &json!({})).unwrap_err().to_string().contains("nothing to undo"));
        // The history is bounded.
        for i in 0..(MAX_UNDO + 20) {
            s.execute("report.setProperties", &json!({"title": format!("t{i}")})).unwrap();
        }
        assert_eq!(s.history().0, MAX_UNDO);
    }
}
