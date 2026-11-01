//! ReportCraft engine (L5): the session and the command registry.
//!
//! Everything is a command: the CLI, the MCP server and (later) the egui interface and the control
//! channel all call `Session::execute(id, params)`. Programmatic calls never open dialogs.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod cmd;
pub mod sample;

use std::panic::{AssertUnwindSafe, catch_unwind};

use reportcraft_core::{Error, RowSet};
use reportcraft_model::Report;
use serde_json::Value as Json;

pub use cmd::{CommandSpec, commands, find_command};

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

/// The open report and the rows it reads.
#[derive(Debug, Clone)]
pub struct Session {
    pub report: Report,
    pub rows: RowSet,
}

impl Default for Session {
    fn default() -> Self {
        Self { report: Report::new("Untitled report"), rows: RowSet::default() }
    }
}

impl Session {
    /// Runs one command. A panic that escapes a command is caught and the session restored as it
    /// was before the command: a last-resort guard, not a licence to panic.
    pub fn execute(&mut self, id: &str, params: &Json) -> EngineResult<Json> {
        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_owned()))?;
        let before = self.clone();
        match catch_unwind(AssertUnwindSafe(|| (spec.run)(self, params))) {
            Ok(result) => result,
            Err(_) => {
                *self = before;
                log::error!("command `{id}` panicked; session restored");
                Err(EngineError::Internal(format!("command `{id}` failed unexpectedly")))
            }
        }
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
        let refused = s.execute("data.loadSqlite", &json!({"path": path, "query": "DELETE FROM r"}));
        assert!(refused.is_err());
        assert_eq!(s.rows.rows.len(), 2, "a refused command leaves the loaded rows as they were");
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
    fn sample_renders_to_html() {
        let mut s = Session::default();
        s.execute("file.openSample", &json!({"name": "rentals"})).unwrap();
        let out = s.execute("report.render", &json!({"format": "html"})).unwrap();
        let html = out.get("html").and_then(Json::as_str).unwrap();
        assert!(html.contains("Lyon") && html.contains("1 / 1"));
    }
}
