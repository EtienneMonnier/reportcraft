//! The ReportCraft designer (L6): a banded canvas of the open report and a live preview of its
//! pages, in egui. Every action goes through an engine command (`Session::execute`), as the CLI
//! and the MCP server do; the designer only shows the session and asks for files.
//!
//! File dialogs belong to the application: the designer asks through [`Dialogs`], which the
//! desktop app implements with the system's dialogs and tests with fixed answers.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod canvas;
mod preview;

use std::path::{Path, PathBuf};

use reportcraft_engine::{Access, Session, find_command};
use reportcraft_layout::Document;
use serde_json::{Value as Json, json};

/// Where the designer asks for files.
pub trait Dialogs {
    /// A file to open, of one of these extensions (without dots).
    fn open(&self, title: &str, extensions: &[&str]) -> Option<PathBuf>;
    /// Where to save, suggesting `name`.
    fn save(&self, title: &str, extension: &str, name: &str) -> Option<PathBuf>;
}

/// No dialog: every question is cancelled (tests, a headless run).
pub struct NoDialogs;

impl Dialogs for NoDialogs {
    fn open(&self, _: &str, _: &[&str]) -> Option<PathBuf> {
        None
    }

    fn save(&self, _: &str, _: &str, _: &str) -> Option<PathBuf> {
        None
    }
}

/// The last rendering of the report, for the revision of the session it was made from.
#[derive(Default)]
struct Preview {
    revision: Option<u64>,
    document: Option<Document>,
    error: Option<String>,
}

/// Loading rows from a saved connection: which one, and the query.
#[derive(Default)]
struct SavedQuery {
    name: String,
    query: String,
}

/// The designer window's state.
pub struct Designer {
    session: Session,
    dialogs: Box<dyn Dialogs>,
    /// The report file, once opened or saved.
    path: Option<PathBuf>,
    /// What the rows came from, for the status bar.
    data: String,
    status: String,
    /// Raised by every command that changes the session; the preview follows it.
    revision: u64,
    preview: Preview,
    /// Canvas pixels per millimetre.
    zoom: f32,
    /// The selected element: band name and index.
    selected: Option<(String, usize)>,
    saved_query: Option<SavedQuery>,
}

impl Designer {
    pub fn new(session: Session, dialogs: Box<dyn Dialogs>) -> Self {
        Self {
            session,
            dialogs,
            path: None,
            data: String::new(),
            status: String::new(),
            revision: 0,
            preview: Preview::default(),
            zoom: 3.5,
            selected: None,
            saved_query: None,
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The status line (the last error, or what the last action did).
    pub fn status(&self) -> &str {
        &self.status
    }

    /// The pages the preview shows, rendering the report again if it changed.
    pub fn preview(&mut self) -> Result<&Document, String> {
        self.refresh_preview();
        match (&self.preview.document, &self.preview.error) {
            (_, Some(e)) => Err(e.clone()),
            (Some(doc), None) => Ok(doc),
            (None, None) => Err("nothing rendered yet".into()),
        }
    }

    /// Runs an engine command; a failure goes to the status line. Commands that change the
    /// session make the preview render again.
    pub fn run(&mut self, id: &str, params: Json) -> Option<Json> {
        match self.session.execute(id, &params) {
            Ok(v) => {
                if find_command(id).is_some_and(|c| c.access == Access::Edit) {
                    self.revision += 1;
                }
                Some(v)
            }
            Err(e) => {
                self.status = e.to_string();
                None
            }
        }
    }

    /// Opens the built-in sample with its data.
    pub fn open_sample(&mut self, name: &str) {
        if let Some(r) = self.run("file.openSample", json!({"name": name})) {
            self.path = None;
            self.selected = None;
            self.data = format!("sample `{name}`, {} rows", r["rows"]);
            self.status = format!("opened the sample `{name}`");
        }
    }

    /// Opens a report file (`*.rcr.json`).
    pub fn open_report(&mut self, path: &Path) {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                if self.run("report.load", json!({"json": text})).is_some() {
                    self.path = Some(path.to_owned());
                    self.selected = None;
                    self.status = format!("opened {}", path.display());
                }
            }
            Err(e) => self.status = format!("{}: {e}", path.display()),
        }
    }

    /// Loads a CSV file as the report's rows.
    pub fn load_csv(&mut self, path: &Path) {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                if let Some(r) = self.run("data.loadCsv", json!({"csv": text})) {
                    self.data = format!("{}, {} rows", path.display(), r["rows"]);
                    self.status = format!("loaded {}", path.display());
                }
            }
            Err(e) => self.status = format!("{}: {e}", path.display()),
        }
    }

    /// Loads rows through a saved connection (its kind decides SQLite or PostgreSQL).
    pub fn load_saved(&mut self, name: &str, query: &str) {
        let kind = self
            .run("data.connections", json!({}))
            .and_then(|r| r["connections"].as_array()?.iter().find(|c| c["name"] == json!(name))?["kind"].as_str().map(str::to_owned));
        let command = match kind.as_deref() {
            Some("sqlite") => "data.loadSqlite",
            Some("postgres") => "data.loadPostgres",
            _ => {
                self.status = format!("no saved connection `{name}`");
                return;
            }
        };
        if let Some(r) = self.run(command, json!({"saved": name, "query": query})) {
            self.data = format!("connection `{name}`, {} rows", r["rows"]);
            self.status = format!("loaded {} rows from `{name}`", r["rows"]);
        }
    }

    /// Saves the report to `path`.
    pub fn save_report(&mut self, path: &Path) {
        let Some(report) = self.run("report.get", json!({})) else { return };
        let text = match serde_json::to_string_pretty(&report) {
            Ok(t) => t,
            Err(e) => {
                self.status = e.to_string();
                return;
            }
        };
        match std::fs::write(path, text) {
            Ok(()) => {
                self.path = Some(path.to_owned());
                self.status = format!("saved {}", path.display());
            }
            Err(e) => self.status = format!("{}: {e}", path.display()),
        }
    }

    /// Exports the report as `format` (`pdf`, `html`, `xlsx`, `docx`) to `path`.
    pub fn export(&mut self, format: &str, path: &Path) {
        let Some(r) = self.run("report.render", json!({"format": format})) else { return };
        let bytes = match format {
            "html" => r["html"].as_str().map(|h| h.as_bytes().to_vec()),
            _ => r[format!("{format}Base64")].as_str().and_then(reportcraft_engine::base64::decode),
        };
        let Some(bytes) = bytes else {
            self.status = format!("the engine gave no {format} file");
            return;
        };
        match std::fs::write(path, bytes) {
            Ok(()) => {
                let warnings: Vec<&str> = r["warnings"].as_array().into_iter().flatten().filter_map(Json::as_str).collect();
                self.status = if warnings.is_empty() {
                    format!("exported {}", path.display())
                } else {
                    format!("exported {} ({})", path.display(), warnings.join("; "))
                };
            }
            Err(e) => self.status = format!("{}: {e}", path.display()),
        }
    }

    fn refresh_preview(&mut self) {
        if self.preview.revision == Some(self.revision) {
            return;
        }
        self.preview.revision = Some(self.revision);
        match self.session.execute("report.render", &json!({"format": "json"})) {
            Ok(v) => match serde_json::from_value::<Document>(v) {
                Ok(doc) => {
                    self.preview.document = Some(doc);
                    self.preview.error = None;
                }
                Err(e) => self.preview.error = Some(e.to_string()),
            },
            Err(e) => self.preview.error = Some(e.to_string()),
        }
    }

    /// One frame of the designer window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("menu").show(ui, |ui| self.menu(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::right("preview").resizable(true).default_size(420.0).show(ui, |ui| {
            ui.heading("Preview");
            self.refresh_preview();
            match (&self.preview.document, &self.preview.error) {
                (_, Some(e)) => {
                    ui.colored_label(egui::Color32::from_rgb(0xcc, 0, 0), e);
                }
                (Some(doc), None) => preview::show(ui, doc),
                (None, None) => {}
            }
        });
        egui::Panel::bottom("properties").resizable(true).default_size(140.0).show(ui, |ui| self.properties(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(self.session.report.title.clone());
                ui.add(egui::Slider::new(&mut self.zoom, 1.0..=5.0).text("zoom"));
            });
            if let Some(hit) = canvas::show(ui, &self.session.report, self.zoom, self.selected.as_ref()) {
                self.selected = Some(hit);
            }
        });
        self.saved_query_window(ui);
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New report").clicked() {
                    self.run("file.new", json!({}));
                    self.path = None;
                    self.selected = None;
                }
                if ui.button("Open sample").clicked() {
                    self.open_sample("rentals");
                }
                if ui.button("Open report…").clicked()
                    && let Some(p) = self.dialogs.open("Open report", &["json"])
                {
                    self.open_report(&p);
                }
                if ui.button("Save report…").clicked() {
                    let name = self
                        .path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map_or_else(|| "report.rcr.json".to_owned(), |n| n.to_string_lossy().into_owned());
                    if let Some(p) = self.dialogs.save("Save report", "json", &name) {
                        self.save_report(&p);
                    }
                }
                ui.separator();
                for (label, format) in [("Export PDF…", "pdf"), ("Export HTML…", "html"), ("Export Excel…", "xlsx"), ("Export Word…", "docx")]
                {
                    if ui.button(label).clicked()
                        && let Some(p) = self.dialogs.save(label.trim_end_matches('…'), format, &format!("report.{format}"))
                    {
                        self.export(format, &p);
                    }
                }
            });
            ui.menu_button("Data", |ui| {
                if ui.button("Load CSV…").clicked()
                    && let Some(p) = self.dialogs.open("Load CSV", &["csv", "txt"])
                {
                    self.load_csv(&p);
                }
                if ui.button("Load from a saved connection…").clicked() {
                    self.saved_query = Some(SavedQuery::default());
                }
            });
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let file = self.path.as_ref().map_or_else(|| "unsaved report".to_owned(), |p| p.display().to_string());
            ui.label(file);
            ui.separator();
            ui.label(if self.data.is_empty() { "no data" } else { &self.data });
            ui.separator();
            ui.label(&self.status);
        });
    }

    /// The selected element, read-only for now (editing comes with R3.2).
    fn properties(&self, ui: &mut egui::Ui) {
        ui.heading("Properties");
        let Some((band, index)) = &self.selected else {
            ui.label("Click an element in the canvas to see its properties.");
            return;
        };
        let element = self.session.report.bands().into_iter().find(|(n, _)| n == band).and_then(|(_, b)| b.elements.get(*index));
        match element.map(serde_json::to_string_pretty) {
            Some(Ok(text)) => {
                ui.label(format!("{band}.elements[{index}]"));
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.monospace(text);
                });
            }
            _ => {
                ui.label("The element is gone.");
            }
        }
    }

    fn saved_query_window(&mut self, ui: &mut egui::Ui) {
        let Some(form) = self.saved_query.as_mut() else { return };
        let names: Vec<String> = match self.session.execute("data.connections", &json!({})) {
            Ok(r) => r["connections"].as_array().into_iter().flatten().filter_map(|c| c["name"].as_str().map(str::to_owned)).collect(),
            Err(_) => Vec::new(),
        };
        let mut open = true;
        let mut load = false;
        egui::Window::new("Load from a saved connection").open(&mut open).show(ui.ctx(), |ui| {
            if names.is_empty() {
                ui.label("No saved connection yet: save one with `reportcraft-cli connections save`.");
                return;
            }
            egui::ComboBox::from_label("Connection").selected_text(form.name.clone()).show_ui(ui, |ui| {
                for n in &names {
                    ui.selectable_value(&mut form.name, n.clone(), n);
                }
            });
            ui.label("Query");
            ui.add(egui::TextEdit::multiline(&mut form.query).code_editor().desired_rows(4));
            load = ui.button("Load").clicked();
        });
        if load {
            let (name, query) = (form.name.clone(), form.query.clone());
            self.load_saved(&name, &query);
            self.saved_query = None;
        } else if !open {
            self.saved_query = None;
        }
    }
}

#[cfg(test)]
mod tests;
