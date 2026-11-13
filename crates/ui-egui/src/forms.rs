//! The Print and Connections windows. They only gather what the person chose; the designer turns
//! it into `file.print`, `data.saveConnection`, `data.deleteConnection` and `data.testConnection`
//! (confirmed: the person clicked the button).

use egui::Ui;
use serde_json::{Value as Json, json};

/// File ▸ Print.
#[derive(Default)]
pub(crate) struct Print {
    /// `None`: the system's default printer.
    pub printer: Option<String>,
    pub printers: Vec<String>,
    pub copies: u32,
    pub pages: String,
    /// `""`: the printer's own setting.
    pub duplex: String,
    pub grayscale: bool,
    pub fit: bool,
    /// What the last check said it would send.
    pub job: String,
}

impl Print {
    pub(crate) fn new(printers: Vec<String>) -> Self {
        Self { printers, copies: 1, ..Self::default() }
    }

    /// The `file.print` parameters; `confirm` sends the job.
    pub(crate) fn params(&self, confirm: bool, values: &std::collections::BTreeMap<String, String>) -> Json {
        let mut p = json!({"copies": self.copies, "grayscale": self.grayscale, "fit": self.fit, "confirm": confirm, "params": values});
        if let Some(printer) = &self.printer {
            p["printer"] = json!(printer);
        }
        if !self.pages.trim().is_empty() {
            p["pages"] = json!(self.pages.trim());
        }
        if !self.duplex.is_empty() {
            p["duplex"] = json!(self.duplex);
        }
        p
    }
}

/// What a window asks for this frame.
#[derive(Debug, PartialEq)]
pub(crate) enum Ask {
    Nothing,
    Check,
    Go,
    Close,
}

pub(crate) fn print(ctx: &egui::Context, p: &mut Print) -> Ask {
    let mut ask = Ask::Nothing;
    let mut open = true;
    egui::Window::new(tl!("Print")).open(&mut open).show(ctx, |ui| {
        egui::Grid::new("print").num_columns(2).show(ui, |ui| {
            ui.label(tl!("Printer"));
            let shown = p.printer.clone().unwrap_or_else(|| tl!("the default printer").to_owned());
            egui::ComboBox::from_id_salt("printer").selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut p.printer, None, tl!("the default printer"));
                for name in &p.printers {
                    ui.selectable_value(&mut p.printer, Some(name.clone()), name);
                }
            });
            ui.end_row();
            ui.label(tl!("Copies"));
            ui.add(egui::DragValue::new(&mut p.copies).range(1..=99));
            ui.end_row();
            ui.label(tl!("Pages"));
            ui.add(egui::TextEdit::singleline(&mut p.pages).hint_text(tl!("all, or 1-3,5")));
            ui.end_row();
            ui.label(tl!("Two-sided"));
            egui::ComboBox::from_id_salt("duplex").selected_text(duplex_name(&p.duplex)).show_ui(ui, |ui| {
                for d in ["", "off", "long", "short"] {
                    ui.selectable_value(&mut p.duplex, d.to_owned(), duplex_name(d));
                }
            });
            ui.end_row();
        });
        ui.checkbox(&mut p.grayscale, tl!("black and white"));
        ui.checkbox(&mut p.fit, tl!("shrink to the paper"));
        if !p.job.is_empty() {
            ui.label(egui::RichText::new(&p.job).monospace().small());
        }
        ui.horizontal(|ui| {
            if ui.button(tl!("Check")).clicked() {
                ask = Ask::Check;
            }
            if ui.button(tl!("Print")).clicked() {
                ask = Ask::Go;
            }
        });
    });
    if open { ask } else { Ask::Close }
}

fn duplex_name(d: &str) -> &'static str {
    match d {
        "off" => tl!("one-sided"),
        "long" => tl!("two-sided, long edge"),
        "short" => tl!("two-sided, short edge"),
        _ => tl!("the printer's setting"),
    }
}

/// Data ▸ Connections: the saved ones, and a form for a new one.
#[derive(Default)]
pub(crate) struct Connections {
    pub list: Vec<(String, String, String)>,
    pub name: String,
    /// `sqlite`, `postgres` or `sqlserver`, as `data.saveConnection` names them.
    pub kind: &'static str,
    pub path: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: String,
    pub password: String,
    /// `""`: the default (`prefer`: encrypted, certificate not checked).
    pub sslmode: String,
}

/// What the Connections window asks for.
#[derive(Debug, PartialEq)]
pub(crate) enum Manage {
    Nothing,
    Browse,
    Save,
    Test(String),
    Delete(String),
    Close,
}

impl Connections {
    pub(crate) fn new(list: Vec<(String, String, String)>) -> Self {
        Self { list, kind: "sqlite", port: 5432, host: "localhost".into(), ..Self::default() }
    }

    fn server(&self) -> bool {
        self.kind != "sqlite"
    }

    /// Picks the kind of database; the port follows it unless one was typed.
    pub(crate) fn set_kind(&mut self, kind: &'static str) {
        let default_port = |k: &str| if k == "sqlserver" { 1433 } else { 5432 };
        if self.port == default_port(self.kind) {
            self.port = default_port(kind);
        }
        self.kind = kind;
    }

    /// The `data.saveConnection` parameters (a password only when one was typed).
    pub(crate) fn params(&self) -> Json {
        let mut p = if self.server() {
            json!({"name": self.name, "kind": self.kind, "host": self.host, "port": self.port, "user": self.user, "database": self.database, "sslmode": self.sslmode})
        } else {
            json!({"name": self.name, "kind": "sqlite", "path": self.path})
        };
        if self.server() && !self.password.is_empty() {
            p["password"] = json!(self.password);
        }
        p["confirm"] = json!(true);
        p
    }
}

pub(crate) fn connections(ctx: &egui::Context, c: &mut Connections) -> Manage {
    let mut ask = Manage::Nothing;
    let mut open = true;
    egui::Window::new(tl!("Connections")).open(&mut open).default_width(460.0).show(ctx, |ui| {
        if c.list.is_empty() {
            ui.label(tl!("No saved connection yet."));
        }
        for (name, kind, target) in &c.list {
            ui.horizontal(|ui| {
                ui.strong(name);
                ui.label(format!("{kind} · {target}"));
                if ui.small_button(tl!("Test")).clicked() {
                    ask = Manage::Test(name.clone());
                }
                if ui.small_button(tl!("Delete")).clicked() {
                    ask = Manage::Delete(name.clone());
                }
            });
        }
        ui.separator();
        ui.heading(tl!("New connection"));
        form(ui, c, &mut ask);
    });
    if open { ask } else { Manage::Close }
}

fn sslmode_name(m: &str, kind: &str) -> &'static str {
    match m {
        // SQL Server always encrypts the login.
        "disable" if kind == "sqlserver" => tl!("the login only"),
        "disable" => tl!("none"),
        "require" => tl!("encrypted"),
        "verify-full" => tl!("encrypted, certificate checked"),
        _ => tl!("encrypted when the server offers it"),
    }
}

fn form(ui: &mut Ui, c: &mut Connections, ask: &mut Manage) {
    egui::Grid::new("connection").num_columns(2).show(ui, |ui| {
        ui.label(tl!("Name"));
        ui.text_edit_singleline(&mut c.name);
        ui.end_row();
        ui.label(tl!("Database"));
        ui.horizontal(|ui| {
            for (kind, label) in [("sqlite", "SQLite"), ("postgres", "PostgreSQL"), ("sqlserver", "SQL Server")] {
                if ui.radio(c.kind == kind, label).clicked() {
                    c.set_kind(kind);
                }
            }
        });
        ui.end_row();
        if c.server() {
            ui.label(tl!("Host"));
            ui.text_edit_singleline(&mut c.host);
            ui.end_row();
            ui.label(tl!("Port"));
            ui.add(egui::DragValue::new(&mut c.port).range(1..=65535));
            ui.end_row();
            ui.label(tl!("User"));
            ui.text_edit_singleline(&mut c.user);
            ui.end_row();
            ui.label(tl!("Database name"));
            ui.text_edit_singleline(&mut c.database);
            ui.end_row();
            ui.label(tl!("Encryption"));
            egui::ComboBox::from_id_salt("sslmode").selected_text(sslmode_name(&c.sslmode, c.kind)).show_ui(ui, |ui| {
                for m in ["", "disable", "require", "verify-full"] {
                    ui.selectable_value(&mut c.sslmode, m.to_owned(), sslmode_name(m, c.kind));
                }
            });
            ui.end_row();
            ui.label(tl!("Password"));
            ui.add(egui::TextEdit::singleline(&mut c.password).password(true).hint_text(tl!("kept in the system keychain")));
            ui.end_row();
        } else {
            ui.label(tl!("File"));
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut c.path);
                if ui.button(tl!("Browse…")).clicked() {
                    *ask = Manage::Browse;
                }
            });
            ui.end_row();
        }
    });
    if ui.button(tl!("Save")).clicked() {
        *ask = Manage::Save;
    }
}
