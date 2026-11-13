//! The ReportCraft designer (L6): a banded canvas of the open report and a live preview of its
//! pages, in egui. Every action goes through an engine command (`Session::execute`), as the CLI
//! and the MCP server do; the designer only shows the session and asks for files.
//!
//! File dialogs belong to the application: the designer asks through [`Dialogs`], which the
//! desktop app implements with the system's dialogs and tests with fixed answers.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[macro_use]
mod i18n;
mod canvas;
mod forms;
mod preview;
mod props;
mod structure;

pub use i18n::{language, set_language};

use std::path::{Path, PathBuf};

use reportcraft_engine::{Access, Session, find_command};
use reportcraft_layout::Document;
use serde_json::{Value as Json, json};

const UNDO: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z);
const REDO: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT), egui::Key::Z);
const REDO_Y: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y);

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
    /// The selected band, when no element is.
    selected_band: Option<String>,
    canvas: canvas::State,
    /// The properties panel's working copy, for the revision it was taken at.
    draft: Option<(u64, Draft)>,
    saved_query: Option<SavedQuery>,
    /// The structure window's working copy, while it is open.
    structure: Option<structure::Draft>,
    /// The values given to the report's parameters, for the preview and the exports.
    param_values: std::collections::BTreeMap<String, String>,
    print: Option<forms::Print>,
    connections: Option<forms::Connections>,
}

/// What the properties panel edits.
enum Draft {
    Element(String, usize, reportcraft_model::Element),
    Band(String, reportcraft_model::Band),
    Report(props::ReportDraft),
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
            selected_band: None,
            canvas: canvas::State::default(),
            draft: None,
            saved_query: None,
            structure: None,
            param_values: std::collections::BTreeMap::new(),
            print: None,
            connections: None,
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

    /// A request from the control channel: an engine command run as an agent would (saved
    /// connections only, no confirmed external action), or `ui.state`. The window shows the
    /// result as if the person had done it.
    pub fn control(&mut self, method: &str, params: &Json) -> Result<Json, String> {
        if method == "ui.state" {
            return Ok(self.state());
        }
        if method == "ui.window" {
            match params.get("name").and_then(Json::as_str) {
                Some("structure") => self.structure = Some(structure::Draft::of(&self.session.report)),
                Some("saved_query") => self.saved_query = Some(SavedQuery::default()),
                Some("print") => self.open_print(),
                Some("connections") => self.open_connections(),
                other => return Err(format!("ui.window: unknown window {other:?} (structure, saved_query)")),
            }
            return Ok(json!({"opened": params["name"]}));
        }
        if find_command(method).is_none() {
            return Err(format!("unknown method `{method}` (an engine command id, ui.state, ui.window, ui.screenshot or app.quit)"));
        }
        let person = self.session.policy;
        self.session.policy = reportcraft_engine::Policy::AGENT;
        let result = self.session.execute(method, params);
        self.session.policy = person;
        match result {
            Ok(v) => {
                if find_command(method).is_some_and(|c| c.access == Access::Edit) {
                    self.revision += 1;
                }
                self.status = format!("{method} (control channel)");
                Ok(v)
            }
            Err(e) => {
                self.status = e.to_string();
                Err(e.to_string())
            }
        }
    }

    /// What the window shows: the file, the data, the selection, the status, the history.
    pub fn state(&self) -> Json {
        let (undo, redo) = self.session.history();
        json!({
            "title": self.session.report.title,
            "path": self.path.as_ref().map(|p| p.display().to_string()),
            "data": self.data,
            "status": self.status,
            "selected": self.selected.as_ref().map(|(b, i)| json!({"band": b, "index": i})),
            "selectedBand": self.selected_band,
            "zoom": self.zoom,
            "undo": undo,
            "redo": redo,
        })
    }

    /// Opens the built-in sample with its data.
    pub fn open_sample(&mut self, name: &str) {
        if let Some(r) = self.run("file.openSample", json!({"name": name})) {
            self.path = None;
            self.selected = None;
            self.data = tf!("sample `{name}`, {n} rows", name = name, n = r["rows"]);
            self.status = tf!("opened the sample `{name}`", name = name);
        }
    }

    /// Opens a report file (`*.rcr.json`).
    pub fn open_report(&mut self, path: &Path) {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                if self.run("report.load", json!({"json": text})).is_some() {
                    self.path = Some(path.to_owned());
                    self.selected = None;
                    self.status = tf!("opened {file}", file = path.display());
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
                    self.data = tf!("{file}, {n} rows", file = path.display(), n = r["rows"]);
                    self.status = tf!("loaded {file}", file = path.display());
                }
            }
            Err(e) => self.status = format!("{}: {e}", path.display()),
        }
    }

    /// Loads rows through a saved connection (its kind decides SQLite, PostgreSQL or SQL Server).
    pub fn load_saved(&mut self, name: &str, query: &str) {
        let kind = self
            .run("data.connections", json!({}))
            .and_then(|r| r["connections"].as_array()?.iter().find(|c| c["name"] == json!(name))?["kind"].as_str().map(str::to_owned));
        let command = match kind.as_deref() {
            Some("sqlite") => "data.loadSqlite",
            Some("postgres") => "data.loadPostgres",
            Some("sqlserver") => "data.loadSqlServer",
            _ => {
                self.status = format!("no saved connection `{name}`");
                return;
            }
        };
        if let Some(r) = self.run(command, json!({"saved": name, "query": query})) {
            self.data = tf!("connection `{name}`, {n} rows", name = name, n = r["rows"]);
            self.status = tf!("loaded {n} rows from `{name}`", name = name, n = r["rows"]);
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
                self.status = tf!("saved {file}", file = path.display());
            }
            Err(e) => self.status = format!("{}: {e}", path.display()),
        }
    }

    /// Exports the report as `format` (`pdf`, `html`, `xlsx`, `docx`) to `path`.
    pub fn export(&mut self, format: &str, path: &Path) {
        let Some(r) = self.run("report.render", json!({"format": format, "params": self.param_values})) else { return };
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
        match self.session.execute("report.render", &json!({"format": "json", "params": self.param_values})) {
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
            ui.heading(tl!("Preview"));
            self.refresh_preview();
            match (&self.preview.document, &self.preview.error) {
                (_, Some(e)) => {
                    ui.colored_label(egui::Color32::from_rgb(0xcc, 0, 0), e);
                }
                (Some(doc), None) => preview::show(ui, doc),
                (None, None) => {}
            }
        });
        egui::Panel::bottom("properties").resizable(true).default_size(190.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.properties(ui));
        });
        egui::Panel::left("fields").resizable(true).default_size(170.0).show(ui, |ui| self.fields(ui));
        // Delete removes the selected element, and Ctrl+Z / Ctrl+Shift+Z (or Ctrl+Y) undo and redo,
        // unless a text box has the keyboard (it undoes its own typing).
        if !ui.ctx().egui_wants_keyboard_input() {
            if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                self.delete_selected();
            }
            if ui.input_mut(|i| i.consume_shortcut(&REDO) || i.consume_shortcut(&REDO_Y)) {
                self.redo();
            } else if ui.input_mut(|i| i.consume_shortcut(&UNDO)) {
                self.undo();
            }
        }
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(self.session.report.title.clone());
                ui.add(egui::Slider::new(&mut self.zoom, 1.0..=5.0).text(tl!("zoom")));
            });
            let events = canvas::show(ui, &self.session.report, self.zoom, self.selected.as_ref(), &mut self.canvas);
            for e in events {
                self.apply(e);
            }
        });
        self.saved_query_window(ui);
        self.structure_window(ui);
        self.print_window(ui);
        self.connections_window(ui);
    }

    fn open_print(&mut self) {
        let printers = self
            .run("file.printers", json!({}))
            .map(|r| r["printers"].as_array().into_iter().flatten().filter_map(|p| p["name"].as_str().map(str::to_owned)).collect());
        self.print = Some(forms::Print::new(printers.unwrap_or_default()));
    }

    fn print_window(&mut self, ui: &mut egui::Ui) {
        let Some(form) = self.print.as_mut() else { return };
        let ask = forms::print(ui.ctx(), form);
        let (confirm, params) = match ask {
            forms::Ask::Nothing => return,
            forms::Ask::Close => {
                self.print = None;
                return;
            }
            forms::Ask::Check => (false, form.params(false, &self.param_values)),
            forms::Ask::Go => (true, form.params(true, &self.param_values)),
        };
        // The person clicked: the window's session prints (a person's policy).
        if let Some(r) = self.run("file.print", params) {
            let command: Vec<&str> = r["command"].as_array().into_iter().flatten().filter_map(Json::as_str).collect();
            let summary = tf!("{n} page(s) of {of}, {copies} copy(ies)", n = r["pages"], of = r["of"], copies = r["copies"]);
            if let Some(form) = self.print.as_mut() {
                form.job = format!("{summary}\n{}", command.join(" "));
            }
            if confirm {
                self.status = r["spooler"].as_str().map_or_else(|| tl!("sent to the printer").to_owned(), |s| s.trim().to_owned());
                self.print = None;
            }
        }
    }

    fn connection_list(&mut self) -> Vec<(String, String, String)> {
        self.run("data.connections", json!({}))
            .map(|r| {
                r["connections"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|c| {
                        let s = |k: &str| c[k].as_str().unwrap_or_default().to_owned();
                        (s("name"), s("kind"), s("target"))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn open_connections(&mut self) {
        let list = self.connection_list();
        self.connections = Some(forms::Connections::new(list));
    }

    fn connections_window(&mut self, ui: &mut egui::Ui) {
        let Some(form) = self.connections.as_mut() else { return };
        match forms::connections(ui.ctx(), form) {
            forms::Manage::Nothing => {}
            forms::Manage::Close => self.connections = None,
            forms::Manage::Browse => {
                if let Some(p) = self.dialogs.open(tl!("Database file"), &["db", "sqlite", "sqlite3"])
                    && let Some(form) = self.connections.as_mut()
                {
                    form.path = p.display().to_string();
                }
            }
            forms::Manage::Save => {
                let params = form.params();
                if self.run("data.saveConnection", params).is_some() {
                    self.status = tl!("connection saved").to_owned();
                    let list = self.connection_list();
                    if let Some(form) = self.connections.as_mut() {
                        form.list = list;
                        form.password.clear();
                    }
                }
            }
            forms::Manage::Test(name) => {
                if let Some(r) = self.run("data.testConnection", json!({"name": name})) {
                    self.status = tf!("{name}: works, {n} table(s)", name = name, n = r["tables"]);
                }
            }
            forms::Manage::Delete(name) => {
                if self.run("data.deleteConnection", json!({"name": name, "confirm": true})).is_some() {
                    let list = self.connection_list();
                    if let Some(form) = self.connections.as_mut() {
                        form.list = list;
                    }
                }
            }
        }
    }

    fn structure_window(&mut self, ui: &mut egui::Ui) {
        let fields = self.field_names();
        let Some(draft) = self.structure.as_mut() else { return };
        match structure::show(ui.ctx(), draft, &fields) {
            structure::Outcome::Open => {}
            structure::Outcome::Close => self.structure = None,
            structure::Outcome::Apply => {
                let params = draft.params();
                if self.run("report.setStructure", params).is_some() {
                    self.structure = None;
                    self.status = tl!("structure applied").to_owned();
                }
            }
        }
    }

    /// Runs `report.validate` and puts its findings in the status line.
    pub fn check(&mut self) {
        let Some(r) = self.run("report.validate", json!({})) else { return };
        let found: Vec<String> = r["diagnostics"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|d| format!("{}: {}", d["location"].as_str().unwrap_or(""), d["message"].as_str().unwrap_or("")))
            .collect();
        self.status = if found.is_empty() { tl!("no problem found").to_owned() } else { found.join(" · ") };
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tl!("File"), |ui| {
                if ui.button(tl!("New report")).clicked() {
                    self.run("file.new", json!({}));
                    self.path = None;
                    self.selected = None;
                }
                if ui.button(tl!("Open sample")).clicked() {
                    self.open_sample("rentals");
                }
                if ui.button(tl!("Open report…")).clicked()
                    && let Some(p) = self.dialogs.open("Open report", &["json"])
                {
                    self.open_report(&p);
                }
                if ui.button(tl!("Save report…")).clicked() {
                    let name = self
                        .path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map_or_else(|| "report.rcr.json".to_owned(), |n| n.to_string_lossy().into_owned());
                    if let Some(p) = self.dialogs.save("Save report", "json", &name) {
                        self.save_report(&p);
                    }
                }
                if ui.button(tl!("Print…")).clicked() {
                    self.open_print();
                }
                ui.separator();
                for (label, format) in [("Export PDF…", "pdf"), ("Export HTML…", "html"), ("Export Excel…", "xlsx"), ("Export Word…", "docx")]
                {
                    if ui.button(tl!(label)).clicked()
                        && let Some(p) = self.dialogs.save(tl!(label).trim_end_matches('…'), format, &format!("report.{format}"))
                    {
                        self.export(format, &p);
                    }
                }
            });
            ui.menu_button(tl!("Edit"), |ui| {
                let (undo, redo) = self.session.history();
                if ui.add_enabled(undo > 0, egui::Button::new(tl!("Undo")).shortcut_text(ui.ctx().format_shortcut(&UNDO))).clicked() {
                    self.undo();
                }
                if ui.add_enabled(redo > 0, egui::Button::new(tl!("Redo")).shortcut_text(ui.ctx().format_shortcut(&REDO))).clicked() {
                    self.redo();
                }
                ui.separator();
                if ui.add_enabled(self.selected.is_some(), egui::Button::new(tl!("Delete element"))).clicked() {
                    self.delete_selected();
                }
            });
            ui.menu_button(tl!("Insert"), |ui| {
                for kind in ["Text", "Field", "Formula", "Total", "Page number", "Line", "Box", "Chart", "Cross-tab", "Barcode"] {
                    if ui.button(tl!(kind)).clicked() {
                        self.insert(kind);
                    }
                }
            });
            ui.menu_button(tl!("Report"), |ui| {
                if ui.button(tl!("Groups, sort, formulas and parameters…")).clicked() {
                    self.structure = Some(structure::Draft::of(&self.session.report));
                }
                if ui.button(tl!("Check the report")).clicked() {
                    self.check();
                }
            });
            ui.menu_button(tl!("View"), |ui| {
                ui.menu_button(tl!("Language"), |ui| {
                    let current = i18n::language();
                    for (code, name) in i18n::languages() {
                        if ui.selectable_label(code == current, name).clicked() {
                            i18n::set_language(&code);
                        }
                    }
                    if i18n::languages().len() == 1 {
                        ui.label(tl!("Other languages come with the craft-i18n catalogs (CRAFT_I18N_DIR at build time)."));
                    }
                });
            });
            ui.menu_button(tl!("Data"), |ui| {
                if ui.button(tl!("Load CSV…")).clicked()
                    && let Some(p) = self.dialogs.open("Load CSV", &["csv", "txt"])
                {
                    self.load_csv(&p);
                }
                if ui.button(tl!("Load from a saved connection…")).clicked() {
                    self.saved_query = Some(SavedQuery::default());
                }
                if ui.button(tl!("Connections…")).clicked() {
                    self.open_connections();
                }
            });
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let file = self.path.as_ref().map_or_else(|| tl!("unsaved report").to_owned(), |p| p.display().to_string());
            ui.label(file);
            ui.separator();
            ui.label(if self.data.is_empty() { tl!("no data") } else { &self.data });
            ui.separator();
            // Shortened to the window's width (hover shows it all), so it never squeezes the panels.
            ui.add(egui::Label::new(&self.status).truncate());
        });
    }

    /// The fields a report can show: the data's columns and the named formulas.
    fn field_names(&self) -> Vec<String> {
        let columns = self.session.rows.columns.iter().map(|c| c.name.clone());
        columns.chain(self.session.report.formulas.iter().map(|f| f.name.clone())).collect()
    }

    /// The field list: drag a field or a parameter onto a band.
    fn fields(&mut self, ui: &mut egui::Ui) {
        ui.heading(tl!("Fields"));
        if self.session.rows.columns.is_empty() {
            ui.label(tl!("Load data (Data menu) to list its fields."));
        }
        let drag = |ui: &mut egui::Ui, key: &str, text: String, what: canvas::FieldDrop| {
            ui.dnd_drag_source(ui.make_persistent_id(("field", key)), what, |ui| {
                ui.label(text);
            });
        };
        for c in &self.session.rows.columns {
            let kind = match c.field_type {
                reportcraft_core::FieldType::Text => tl!("text"),
                reportcraft_core::FieldType::Integer => tl!("integer"),
                reportcraft_core::FieldType::Number => tl!("number"),
                reportcraft_core::FieldType::Bool => tl!("yes/no"),
            };
            drag(ui, &c.name, format!("{}  ·  {kind}", c.name), canvas::FieldDrop::Field(c.name.clone()));
        }
        if !self.session.report.formulas.is_empty() {
            ui.separator();
            ui.label(tl!("Formulas"));
            for f in &self.session.report.formulas {
                drag(ui, &f.name, format!("{} = {}", f.name, f.expression), canvas::FieldDrop::Field(f.name.clone()));
            }
        }
        if !self.session.report.parameters.is_empty() {
            ui.separator();
            ui.label(tl!("Parameters"));
            let mut changed = false;
            for p in &self.session.report.parameters {
                drag(
                    ui,
                    &format!("param:{}", p.name),
                    if p.label.is_empty() { p.name.clone() } else { p.label.clone() },
                    canvas::FieldDrop::Parameter(p.name.clone()),
                );
                let value = self.param_values.entry(p.name.clone()).or_default();
                let hint = p.default.clone().unwrap_or_else(|| match p.kind {
                    reportcraft_model::ParamType::Date => "YYYY-MM-DD".into(),
                    reportcraft_model::ParamType::DateRange => "YYYY-MM-DD..YYYY-MM-DD".into(),
                    _ => String::new(),
                });
                changed |= props::finished(&ui.add(egui::TextEdit::singleline(value).hint_text(hint)));
            }
            // An empty box means no value given (the default applies).
            self.param_values.retain(|_, v| !v.trim().is_empty());
            if changed {
                self.revision += 1;
            }
        }
    }

    /// Turns what happened on the canvas into commands.
    fn apply(&mut self, event: canvas::Event) {
        match event {
            canvas::Event::Select(band, index) => {
                self.selected = Some((band, index));
                self.selected_band = None;
            }
            canvas::Event::SelectBand(band) => {
                self.selected = None;
                self.selected_band = Some(band);
            }
            canvas::Event::Move { band, index, to, x_mm, y_mm } => {
                let Some(mut el) = self.element(&band, index) else { return };
                el.x_mm = x_mm;
                el.y_mm = y_mm;
                let mut params = json!({"band": band, "index": index, "element": el});
                if to != band {
                    params["move_to"] = json!(to);
                }
                if let Some(r) = self.run("report.setElement", params) {
                    let index = r["index"].as_u64().and_then(|i| usize::try_from(i).ok()).unwrap_or(index);
                    self.selected = Some((r["band"].as_str().unwrap_or(&band).to_owned(), index));
                }
            }
            canvas::Event::Resize { band, index, width_mm, height_mm } => {
                let Some(mut el) = self.element(&band, index) else { return };
                el.width_mm = width_mm;
                el.height_mm = height_mm;
                self.run("report.setElement", json!({"band": band, "index": index, "element": el}));
            }
            canvas::Event::Drop { band, x_mm, y_mm, what } => {
                let el = what.element(x_mm, y_mm);
                if let Some(r) = self.run("report.addElement", json!({"band": band, "element": el})) {
                    let index = r["index"].as_u64().and_then(|i| usize::try_from(i).ok()).unwrap_or(0);
                    self.selected = Some((band, index));
                    self.selected_band = None;
                }
            }
        }
    }

    fn element(&self, band: &str, index: usize) -> Option<reportcraft_model::Element> {
        self.session.report.bands().into_iter().find(|(n, _)| n == band).and_then(|(_, b)| b.elements.get(index).cloned())
    }

    /// Adds an element of `kind` to the selected band (the detail band when none is).
    pub fn insert(&mut self, kind: &str) {
        let band = self.selected.as_ref().map(|(b, _)| b.clone()).or_else(|| self.selected_band.clone()).unwrap_or_else(|| "detail".to_owned());
        let first = self.field_names().first().cloned().unwrap_or_default();
        let mut el = canvas::FieldDrop::Field(first.clone()).element(0.0, 0.0);
        el.content = props::content_of(kind, &reportcraft_model::Content::Text { text: kind.to_owned() }, &first);
        if matches!(el.content, reportcraft_model::Content::Line { .. }) {
            el.height_mm = 0.0;
        }
        if matches!(el.content, reportcraft_model::Content::Chart { .. } | reportcraft_model::Content::Crosstab { .. }) {
            (el.width_mm, el.height_mm) = (120.0, 50.0);
        }
        if matches!(el.content, reportcraft_model::Content::Barcode { .. }) {
            (el.width_mm, el.height_mm) = (50.0, 15.0);
        }
        if let Some(r) = self.run("report.addElement", json!({"band": band, "element": el})) {
            let index = r["index"].as_u64().and_then(|i| usize::try_from(i).ok()).unwrap_or(0);
            self.selected = Some((band, index));
            self.selected_band = None;
        }
    }

    /// Puts back the report as it was before the last change.
    pub fn undo(&mut self) {
        if self.run("edit.undo", json!({})).is_some() {
            self.status = tl!("undone").to_owned();
        }
    }

    /// Does again what was undone.
    pub fn redo(&mut self) {
        if self.run("edit.redo", json!({})).is_some() {
            self.status = tl!("redone").to_owned();
        }
    }

    /// Deletes the selected element.
    pub fn delete_selected(&mut self) {
        if let Some((band, index)) = self.selected.take()
            && self.run("report.removeElement", json!({"band": band, "index": index})).is_none()
        {
            self.selected = Some((band, index));
        }
    }

    /// The working copy for what is selected, taken again after every change to the session.
    fn sync_draft(&mut self) {
        let wanted = match (&self.selected, &self.selected_band, &self.draft) {
            (Some((b, i)), _, Some((rev, Draft::Element(db, di, _)))) => *rev != self.revision || b != db || i != di,
            (None, Some(b), Some((rev, Draft::Band(db, _)))) => *rev != self.revision || b != db,
            (None, None, Some((rev, Draft::Report(_)))) => *rev != self.revision,
            _ => true,
        };
        if !wanted {
            return;
        }
        let r = &self.session.report;
        let draft = match (&self.selected, &self.selected_band) {
            (Some((b, i)), _) => self.element(b, *i).map(|el| Draft::Element(b.clone(), *i, el)),
            (None, Some(b)) => r.bands().into_iter().find(|(n, _)| n == b).map(|(_, band)| Draft::Band(b.clone(), band.clone())),
            (None, None) => Some(Draft::Report(props::ReportDraft {
                title: r.title.clone(),
                locale: r.locale.clone(),
                paper: r.page.paper.map_or_else(|| "custom".to_owned(), |p| format!("{p:?}")),
                landscape: r.page.orientation == reportcraft_model::Orientation::Landscape,
                margin_mm: r.page.margin_mm,
            })),
        };
        if draft.is_none() {
            self.selected = None;
            self.selected_band = None;
        }
        self.draft = draft.map(|d| (self.revision, d));
    }

    /// The properties of what is selected, editable; each finished edit is one command.
    fn properties(&mut self, ui: &mut egui::Ui) {
        self.sync_draft();
        let fields = self.field_names();
        let report = &self.session.report;
        let Some((_, draft)) = self.draft.as_mut() else { return };
        let command = match draft {
            Draft::Element(band, index, el) => {
                ui.horizontal(|ui| {
                    ui.heading(tl!("Element"));
                    ui.label(format!("{} · {index}", canvas::band_title(report, band)));
                });
                props::element(ui, el, &fields).then(|| ("report.setElement", json!({"band": band, "index": index, "element": el})))
            }
            Draft::Band(band, b) => {
                ui.heading(format!("{} · {}", tl!("Band"), canvas::band_title(report, band)));
                props::band(ui, b).then(|| {
                    let formula = |f: &Option<String>| f.clone().unwrap_or_default();
                    (
                        "report.setBand",
                        json!({"band": band, "height_mm": b.height_mm, "visible_if": formula(&b.visible_if), "page_break_before_if": formula(&b.page_break_before_if), "keep_with_next": b.keep_with_next}),
                    )
                })
            }
            Draft::Report(r) => {
                ui.heading(tl!("Report"));
                ui.label(tl!("Click an element or a band's name to edit it; drag fields from the list onto a band."));
                props::report(ui, r).then(|| {
                    let mut page = json!({"orientation": if r.landscape { "landscape" } else { "portrait" }, "margin_mm": r.margin_mm});
                    if r.paper != "custom" {
                        page["paper"] = json!(r.paper);
                    }
                    ("report.setProperties", json!({"title": r.title, "locale": r.locale, "page": page}))
                })
            }
        };
        if let Some((id, params)) = command
            && self.run(id, params).is_none()
        {
            // Refused (an invalid value): show the report as it is.
            self.draft = None;
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
        egui::Window::new(tl!("Load from a saved connection")).open(&mut open).show(ui.ctx(), |ui| {
            if names.is_empty() {
                ui.label(tl!("No saved connection yet: save one with `reportcraft-cli connections save`."));
                return;
            }
            egui::ComboBox::from_label(tl!("Connection")).selected_text(form.name.clone()).show_ui(ui, |ui| {
                for n in &names {
                    ui.selectable_value(&mut form.name, n.clone(), n);
                }
            });
            ui.label(tl!("Query"));
            ui.add(egui::TextEdit::multiline(&mut form.query).code_editor().desired_rows(4));
            load = ui.button(tl!("Load")).clicked();
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
