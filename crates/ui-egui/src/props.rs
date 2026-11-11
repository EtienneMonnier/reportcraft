//! The properties panel: editors for the selected element, the selected band or the report. An
//! editor works on a copy and says when an edit is finished (focus left, drag stopped, a choice
//! made), so the designer sends one command per edit rather than one per frame.

use egui::{Response, Ui};
use reportcraft_model::{AggregateFn, Align, Band, Color, Content, Element, Format};

/// Whether an edit made through `r` is finished and should be applied.
pub(crate) fn finished(r: &Response) -> bool {
    r.lost_focus() || r.drag_stopped() || (r.changed() && !r.has_focus() && !r.dragged())
}

/// The kinds of content the panel can switch between, with a starting value for each.
const KINDS: &[&str] = &["Text", "Field", "Formula", "Total", "Parameter", "Page number", "Line", "Box", "Chart", "Barcode"];

fn kind_of(c: &Content) -> &'static str {
    match c {
        Content::Text { .. } => "Text",
        Content::Field { .. } => "Field",
        Content::Formula { .. } => "Formula",
        Content::Aggregate { .. } => "Total",
        Content::Parameter { .. } => "Parameter",
        Content::PageNumber => "Page number",
        Content::Line { .. } => "Line",
        Content::Rect { .. } => "Box",
        Content::RefreshedAt => "Refreshed at",
        Content::PrintedAt => "Printed at",
        Content::Barcode { .. } => "Barcode",
        Content::Subreport { .. } => "Subreport",
        Content::Chart { .. } => "Chart",
    }
}

/// The content of a kind, keeping the field or text it had when that makes sense.
pub(crate) fn content_of(kind: &str, from: &Content, first_field: &str) -> Content {
    let field = match from {
        Content::Field { field } | Content::Aggregate { field, .. } => field.clone(),
        _ => first_field.to_owned(),
    };
    match kind {
        "Text" => Content::Text { text: crate::canvas::label(from) },
        "Field" => Content::Field { field },
        "Formula" => Content::Formula {
            expression: match from {
                Content::Field { field } => format!("[{field}]"),
                Content::Formula { expression } => expression.clone(),
                // A formula must parse: start from the first field, or a number.
                _ if !first_field.is_empty() => format!("[{first_field}]"),
                _ => "0".to_owned(),
            },
        },
        "Total" => Content::Aggregate { function: AggregateFn::Sum, field },
        "Parameter" => Content::Parameter { name: String::new() },
        "Page number" => Content::PageNumber,
        "Line" => Content::Line { thickness_mm: 0.3, color: "#000000".into() },
        "Box" => Content::Rect { border_mm: 0.3, border_color: "#000000".into(), fill: None },
        "Barcode" => Content::Barcode {
            symbology: reportcraft_model::Symbology::Code128,
            field: Some(field.clone()),
            text: None,
            show_text: true,
            ec_level: reportcraft_model::EcLevel::M,
        },
        "Chart" => Content::Chart {
            chart: reportcraft_model::ChartKind::Bar,
            category: first_field.to_owned(),
            value: field,
            function: AggregateFn::Sum,
            title: None,
        },
        _ => from.clone(),
    }
}

/// A text box for an optional formula: empty means none.
fn optional_formula(ui: &mut Ui, label: &str, value: &mut Option<String>) -> bool {
    let mut text = value.clone().unwrap_or_default();
    let r = ui.horizontal(|ui| {
        ui.label(tl!(label));
        ui.add(egui::TextEdit::singleline(&mut text).code_editor().hint_text(tl!("formula, e.g. [amount] < 0")).desired_width(f32::INFINITY))
    });
    *value = (!text.trim().is_empty()).then_some(text);
    finished(&r.inner)
}

/// A checkbox and a colour button for an optional colour.
fn optional_colour(ui: &mut Ui, label: &str, value: &mut Option<Color>) -> bool {
    let mut on = value.is_some();
    let mut rgb = value.map_or([0, 0, 0], |c| [c.0, c.1, c.2]);
    let mut done = false;
    ui.horizontal(|ui| {
        done |= ui.checkbox(&mut on, tl!(label)).changed();
        if on {
            let r = ui.color_edit_button_srgb(&mut rgb);
            done |= r.changed() && !r.dragged();
            done |= r.drag_stopped();
        }
    });
    *value = on.then_some(Color(rgb[0], rgb[1], rgb[2]));
    done
}

fn length(ui: &mut Ui, label: &str, value: &mut f64) -> bool {
    ui.label(tl!(label));
    finished(&ui.add(egui::DragValue::new(value).speed(0.5).range(0.0..=5000.0).suffix(" mm")))
}

/// The element editor; `true` when an edit is finished.
pub(crate) fn element(ui: &mut Ui, el: &mut Element, fields: &[String]) -> bool {
    let mut done = false;
    egui::Grid::new("geometry").num_columns(8).show(ui, |ui| {
        done |= length(ui, "x", &mut el.x_mm);
        done |= length(ui, "y", &mut el.y_mm);
        done |= length(ui, "width", &mut el.width_mm);
        done |= length(ui, "height", &mut el.height_mm);
        ui.end_row();
    });
    done |= content(ui, &mut el.content, fields);
    if !matches!(el.content, Content::Line { .. } | Content::Rect { .. }) {
        done |= style(ui, el);
        done |= format(ui, &mut el.format);
        done |= optional_formula(ui, "Colour formula", &mut el.color_formula);
        done |= optional_formula(ui, "Background formula", &mut el.background_formula);
    }
    done |= optional_formula(ui, "Show only if", &mut el.visible_if);
    done
}

fn content(ui: &mut Ui, c: &mut Content, fields: &[String]) -> bool {
    let mut done = false;
    let current = kind_of(c);
    let first = fields.first().map_or("", String::as_str);
    ui.horizontal(|ui| {
        ui.label(tl!("Shows"));
        egui::ComboBox::from_id_salt("kind").selected_text(tl!(current)).show_ui(ui, |ui| {
            for k in KINDS {
                if ui.selectable_label(*k == current, tl!(k)).clicked() && *k != current {
                    *c = content_of(k, c, first);
                    done = true;
                }
            }
        });
    });
    let field_combo = |ui: &mut Ui, id: &str, field: &mut String| {
        let mut chosen = false;
        egui::ComboBox::from_id_salt(id.to_owned()).selected_text(field.clone()).show_ui(ui, |ui| {
            for f in fields {
                chosen |= ui.selectable_value(field, f.clone(), f).clicked();
            }
        });
        chosen
    };
    match c {
        Content::Text { text } => done |= finished(&ui.text_edit_singleline(text)),
        Content::Field { field } => done |= ui.horizontal(|ui| field_combo(ui, "field", field)).inner,
        Content::Formula { expression } => {
            done |= finished(&ui.add(egui::TextEdit::multiline(expression).code_editor().desired_rows(2).desired_width(f32::INFINITY)));
        }
        Content::Aggregate { function, field } => {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("function").selected_text(crate::canvas::aggregate_name(*function)).show_ui(ui, |ui| {
                    for f in [AggregateFn::Sum, AggregateFn::Count, AggregateFn::Avg, AggregateFn::Min, AggregateFn::Max] {
                        done |= ui.selectable_value(function, f, crate::canvas::aggregate_name(f)).clicked();
                    }
                });
                ui.label(tl!("of"));
                done |= field_combo(ui, "aggregate field", field);
            });
        }
        Content::Parameter { name } => done |= finished(&ui.text_edit_singleline(name)),
        Content::Barcode { symbology, field, text, show_text, ec_level } => {
            use reportcraft_model::{EcLevel, Symbology};
            ui.horizontal(|ui| {
                for (s, label) in [(Symbology::Code128, "Code 128"), (Symbology::Ean13, "EAN-13"), (Symbology::Qr, "QR code")] {
                    done |= ui.radio_value(symbology, s, label).changed();
                }
            });
            ui.horizontal(|ui| {
                let mut from_field = field.is_some();
                let a = ui.radio_value(&mut from_field, true, tl!("from a field")).changed();
                let b = ui.radio_value(&mut from_field, false, tl!("fixed text")).changed();
                done |= a || b;
                match (from_field, field.is_some()) {
                    (true, false) => {
                        *field = Some(first.to_owned());
                        *text = None;
                    }
                    (false, true) => {
                        *field = None;
                        *text = Some(String::new());
                    }
                    _ => {}
                }
                if let Some(f) = field.as_mut() {
                    done |= field_combo(ui, "barcode field", f);
                }
                if let Some(t) = text.as_mut() {
                    done |= finished(&ui.text_edit_singleline(t));
                }
            });
            ui.horizontal(|ui| {
                if *symbology == Symbology::Qr {
                    ui.label(tl!("error correction"));
                    for (l, label) in [(EcLevel::L, "L"), (EcLevel::M, "M"), (EcLevel::Q, "Q"), (EcLevel::H, "H")] {
                        done |= ui.radio_value(ec_level, l, label).changed();
                    }
                } else {
                    done |= ui.checkbox(show_text, tl!("show the text")).changed();
                }
            });
        }
        Content::Chart { chart, category, value, function, title } => {
            ui.horizontal(|ui| {
                for (k, label) in [
                    (reportcraft_model::ChartKind::Bar, "bars"),
                    (reportcraft_model::ChartKind::Line, "line"),
                    (reportcraft_model::ChartKind::Pie, "pie"),
                ] {
                    done |= ui.radio_value(chart, k, tl!(label)).changed();
                }
            });
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("chart function").selected_text(crate::canvas::aggregate_name(*function)).show_ui(ui, |ui| {
                    for f in [AggregateFn::Sum, AggregateFn::Count, AggregateFn::Avg, AggregateFn::Min, AggregateFn::Max] {
                        done |= ui.selectable_value(function, f, crate::canvas::aggregate_name(f)).clicked();
                    }
                });
                ui.label(tl!("of"));
                done |= field_combo(ui, "chart value", value);
                ui.label(tl!("by"));
                done |= field_combo(ui, "chart category", category);
            });
            let mut t = title.clone().unwrap_or_default();
            ui.horizontal(|ui| {
                ui.label(tl!("Title"));
                done |= finished(&ui.text_edit_singleline(&mut t));
            });
            *title = (!t.is_empty()).then_some(t);
        }
        Content::Line { thickness_mm, color } => {
            ui.horizontal(|ui| {
                ui.label(tl!("thickness"));
                done |= finished(&ui.add(egui::DragValue::new(thickness_mm).speed(0.1).range(0.0..=50.0).suffix(" mm")));
                done |= colour_text(ui, color);
            });
        }
        Content::Rect { border_mm, border_color, fill } => {
            ui.horizontal(|ui| {
                ui.label(tl!("border"));
                done |= finished(&ui.add(egui::DragValue::new(border_mm).speed(0.1).range(0.0..=50.0).suffix(" mm")));
                done |= colour_text(ui, border_color);
                let mut filled = fill.clone().and_then(|f| Color::parse(&f));
                done |= optional_colour(ui, "fill", &mut filled);
                *fill = filled.map(|c| c.to_string());
            });
        }
        _ => {}
    }
    done
}

/// A colour kept as text (lines, boxes), edited with a colour button.
fn colour_text(ui: &mut Ui, text: &mut String) -> bool {
    let mut c = Color::parse(text).map_or([0, 0, 0], |c| [c.0, c.1, c.2]);
    let r = ui.color_edit_button_srgb(&mut c);
    *text = Color(c[0], c[1], c[2]).to_string();
    (r.changed() && !r.dragged()) || r.drag_stopped()
}

fn style(ui: &mut Ui, el: &mut Element) -> bool {
    let mut done = false;
    ui.horizontal(|ui| {
        ui.label(tl!("size"));
        done |= finished(&ui.add(egui::DragValue::new(&mut el.style.size_pt).speed(0.5).range(1.0..=400.0).suffix(" pt")));
        done |= ui.checkbox(&mut el.style.bold, tl!("bold")).changed();
        for (a, label) in [(Align::Left, "left"), (Align::Center, "centre"), (Align::Right, "right")] {
            done |= ui.radio_value(&mut el.style.align, a, tl!(label)).changed();
        }
    });
    ui.horizontal(|ui| {
        done |= optional_colour(ui, "colour", &mut el.style.color);
        done |= optional_colour(ui, "background", &mut el.style.background);
    });
    done
}

fn format(ui: &mut Ui, f: &mut Option<Format>) -> bool {
    let mut done = false;
    let names = ["plain", "number", "currency", "percent", "date"];
    let current = match f {
        None => "plain",
        Some(Format::Number { .. }) => "number",
        Some(Format::Currency { .. }) => "currency",
        Some(Format::Percent { .. }) => "percent",
        Some(Format::Date { .. }) => "date",
    };
    ui.horizontal(|ui| {
        ui.label(tl!("Format"));
        egui::ComboBox::from_id_salt("format").selected_text(tl!(current)).show_ui(ui, |ui| {
            for n in names {
                if ui.selectable_label(n == current, tl!(n)).clicked() && n != current {
                    *f = match n {
                        "number" => Some(Format::Number { decimals: 2, grouping: true }),
                        "currency" => Some(Format::Currency { symbol: "€".into(), decimals: 2, symbol_after: true }),
                        "percent" => Some(Format::Percent { decimals: 1 }),
                        "date" => Some(Format::Date { pattern: None }),
                        _ => None,
                    };
                    done = true;
                }
            }
        });
        let decimals = |ui: &mut Ui, d: &mut u8| finished(&ui.add(egui::DragValue::new(d).range(0..=10).prefix(format!("{} ", tl!("decimals")))));
        match f {
            Some(Format::Number { decimals: d, grouping }) => {
                done |= decimals(ui, d);
                done |= ui.checkbox(grouping, "thousands").changed();
            }
            Some(Format::Currency { symbol, decimals: d, symbol_after }) => {
                done |= finished(&ui.add(egui::TextEdit::singleline(symbol).desired_width(30.0)));
                done |= decimals(ui, d);
                done |= ui.checkbox(symbol_after, "symbol after").changed();
            }
            Some(Format::Percent { decimals: d }) => done |= decimals(ui, d),
            Some(Format::Date { pattern }) => {
                let mut p = pattern.clone().unwrap_or_default();
                done |= finished(&ui.add(egui::TextEdit::singleline(&mut p).hint_text(tl!("the locale's date, or %d/%m/%Y")).desired_width(140.0)));
                *pattern = (!p.trim().is_empty()).then_some(p);
            }
            None => {}
        }
    });
    done
}

/// The band editor; `true` when an edit is finished.
pub(crate) fn band(ui: &mut Ui, b: &mut Band) -> bool {
    let mut done = false;
    ui.horizontal(|ui| {
        done |= length(ui, "height", &mut b.height_mm);
        done |= ui.checkbox(&mut b.keep_with_next, tl!("keep with next")).changed();
    });
    done |= optional_formula(ui, "Show only if", &mut b.visible_if);
    done |= optional_formula(ui, "New page if", &mut b.page_break_before_if);
    done
}

/// What the report editor changes.
pub(crate) struct ReportDraft {
    pub title: String,
    pub locale: String,
    pub paper: String,
    pub landscape: bool,
    pub margin_mm: f64,
}

/// The report editor; `true` when an edit is finished.
pub(crate) fn report(ui: &mut Ui, r: &mut ReportDraft) -> bool {
    let mut done = false;
    ui.horizontal(|ui| {
        ui.label(tl!("Title"));
        done |= finished(&ui.text_edit_singleline(&mut r.title));
        ui.label(tl!("Language"));
        egui::ComboBox::from_id_salt("locale").selected_text(r.locale.clone()).show_ui(ui, |ui| {
            for l in reportcraft_model::LOCALES {
                done |= ui.selectable_value(&mut r.locale, l.tag.to_owned(), l.tag).clicked();
            }
        });
    });
    ui.horizontal(|ui| {
        ui.label(tl!("Paper"));
        egui::ComboBox::from_id_salt("paper").selected_text(tl!(r.paper.as_str())).show_ui(ui, |ui| {
            for p in ["A4", "A3", "A5", "Letter", "Legal"] {
                done |= ui.selectable_value(&mut r.paper, p.to_owned(), p).clicked();
            }
        });
        done |= ui.checkbox(&mut r.landscape, tl!("landscape")).changed();
        done |= length(ui, "margins", &mut r.margin_mm);
    });
    done
}
