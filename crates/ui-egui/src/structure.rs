//! The structure window: the report's groups, sort, named formulas and parameters, edited on a
//! copy and applied in one command (`report.setStructure`), which checks them as a whole (a
//! formula's circle, an unknown parameter type).

use egui::Ui;
use reportcraft_model::{FormulaField, Group, ParamType, Parameter, Report, SortKey};

/// The window's working copy.
pub(crate) struct Draft {
    pub groups: Vec<Group>,
    pub sort: Vec<SortKey>,
    pub formulas: Vec<FormulaField>,
    pub parameters: Vec<Parameter>,
}

impl Draft {
    pub(crate) fn of(r: &Report) -> Self {
        Self { groups: r.groups.clone(), sort: r.sort.clone(), formulas: r.formulas.clone(), parameters: r.parameters.clone() }
    }

    pub(crate) fn params(&self) -> serde_json::Value {
        serde_json::json!({"groups": self.groups, "sort": self.sort, "formulas": self.formulas, "parameters": self.parameters})
    }
}

/// What the window asks for this frame.
#[derive(Debug, PartialEq)]
pub(crate) enum Outcome {
    Open,
    Apply,
    Close,
}

const TYPES: &[(ParamType, &str)] = &[
    (ParamType::Text, "text"),
    (ParamType::Integer, "integer"),
    (ParamType::Number, "number"),
    (ParamType::Boolean, "yes/no"),
    (ParamType::Date, "date"),
    (ParamType::DateRange, "date range"),
];

fn field_combo(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: &mut String, fields: &[String]) {
    egui::ComboBox::from_id_salt(id).selected_text(value.clone()).show_ui(ui, |ui| {
        for f in fields {
            ui.selectable_value(value, f.clone(), f);
        }
    });
}

/// Draws the window; says whether to keep it open, apply the draft or close it.
pub(crate) fn show(ctx: &egui::Context, draft: &mut Draft, fields: &[String]) -> Outcome {
    let mut outcome = Outcome::Open;
    let mut open = true;
    let first = fields.first().cloned().unwrap_or_default();
    egui::Window::new(tl!("Groups, sort, formulas and parameters")).open(&mut open).default_width(560.0).show(ctx, |ui| {
        egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
            ui.heading(tl!("Groups"));
            let mut remove = None;
            for (i, g) in draft.groups.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("{}", i + 1));
                    field_combo(ui, ("group", i), &mut g.field, fields);
                    ui.checkbox(&mut g.keep_together, tl!("keep together"));
                    ui.checkbox(&mut g.repeat_header, tl!("repeat header"));
                    ui.checkbox(&mut g.page_break_before, tl!("new page"));
                    if ui.button("×").on_hover_text(tl!("Remove")).clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                draft.groups.remove(i);
            }
            if ui.button(tl!("Add group")).clicked() {
                draft.groups.push(Group {
                    field: first.clone(),
                    keep_together: false,
                    repeat_header: false,
                    page_break_before: false,
                    header: reportcraft_model::Band { height_mm: 7.0, ..Default::default() },
                    footer: reportcraft_model::Band { height_mm: 7.0, ..Default::default() },
                });
            }
            ui.separator();
            ui.heading(tl!("Sort"));
            let mut remove = None;
            for (i, k) in draft.sort.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    field_combo(ui, ("sort", i), &mut k.field, fields);
                    ui.checkbox(&mut k.descending, tl!("descending"));
                    if ui.button("×").on_hover_text(tl!("Remove")).clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                draft.sort.remove(i);
            }
            if ui.button(tl!("Add sort key")).clicked() {
                draft.sort.push(SortKey { field: first.clone(), descending: false });
            }
            ui.separator();
            ui.heading(tl!("Formulas"));
            let mut remove = None;
            for (i, f) in draft.formulas.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(100.0).hint_text(tl!("name")));
                    ui.label("=");
                    ui.add(egui::TextEdit::singleline(&mut f.expression).code_editor().desired_width(320.0));
                    if ui.button("×").on_hover_text(tl!("Remove")).clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                draft.formulas.remove(i);
            }
            if ui.button(tl!("Add formula")).clicked() {
                let n = draft.formulas.len() + 1;
                draft
                    .formulas
                    .push(FormulaField { name: format!("formula{n}"), expression: if first.is_empty() { "0".into() } else { format!("[{first}]") } });
            }
            ui.separator();
            ui.heading(tl!("Parameters"));
            let mut remove = None;
            for (i, p) in draft.parameters.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut p.name).desired_width(90.0).hint_text(tl!("name")));
                    ui.add(egui::TextEdit::singleline(&mut p.label).desired_width(120.0).hint_text(tl!("label")));
                    let current = TYPES.iter().find(|(t, _)| *t == p.kind).map_or("text", |(_, n)| n);
                    egui::ComboBox::from_id_salt(("type", i)).selected_text(tl!(current)).show_ui(ui, |ui| {
                        for (t, n) in TYPES {
                            ui.selectable_value(&mut p.kind, *t, tl!(n));
                        }
                    });
                    let mut default = p.default.clone().unwrap_or_default();
                    ui.add(egui::TextEdit::singleline(&mut default).desired_width(90.0).hint_text(tl!("default")));
                    p.default = (!default.is_empty()).then_some(default);
                    ui.checkbox(&mut p.required, tl!("required"));
                    if ui.button("×").on_hover_text(tl!("Remove")).clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                draft.parameters.remove(i);
            }
            if ui.button(tl!("Add parameter")).clicked() {
                let n = draft.parameters.len() + 1;
                draft.parameters.push(Parameter::text(format!("param{n}"), String::new(), None));
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button(tl!("Apply")).clicked() {
                outcome = Outcome::Apply;
            }
            if ui.button(tl!("Cancel")).clicked() {
                outcome = Outcome::Close;
            }
        });
    });
    if !open { Outcome::Close } else { outcome }
}
