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
    fn sample_renders_to_html() {
        let mut s = Session::default();
        s.execute("file.openSample", &json!({"name": "rentals"})).unwrap();
        let out = s.execute("report.render", &json!({"format": "html"})).unwrap();
        let html = out.get("html").and_then(Json::as_str).unwrap();
        assert!(html.contains("Lyon") && html.contains("1 / 1"));
    }
}
