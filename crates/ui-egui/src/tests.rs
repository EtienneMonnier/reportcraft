use std::path::PathBuf;

use reportcraft_engine::Session;
use reportcraft_model::Content;

use crate::{Designer, NoDialogs, canvas, preview};

/// Runs `frames` frames of the designer without a window and returns how many shapes it painted.
fn frames(designer: &mut Designer, frames: usize) -> usize {
    let ctx = egui::Context::default();
    let mut painted = 0;
    for _ in 0..frames {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| designer.ui(ui));
        painted = out.shapes.len();
        // No renderer here to take the font atlas.
        out.textures_delta.clear();
    }
    painted
}

#[test]
fn the_sample_shows_on_the_canvas_and_in_the_preview() {
    let mut d = Designer::new(Session::default(), Box::new(NoDialogs));
    d.open_sample("rentals");
    assert!(d.status().contains("rentals"), "{}", d.status());
    let painted = frames(&mut d, 3);
    assert!(painted > 50, "bands, elements and the preview page: {painted} shapes");
    let doc = d.preview().map(|doc| doc.pages.len());
    assert_eq!(doc, Ok(1));
}

#[test]
fn an_empty_report_and_a_broken_render_still_draw() {
    let mut d = Designer::new(Session::default(), Box::new(NoDialogs));
    assert!(frames(&mut d, 2) > 0);
    // A required parameter without a value: the preview says why it can't render.
    d.open_sample("rentals");
    let mut report = d.session().report.clone();
    report.parameters.push(reportcraft_model::Parameter { required: true, ..reportcraft_model::Parameter::text("needed", "", None) });
    let json = serde_json::to_string(&report).unwrap_or_default();
    d.run("report.load", serde_json::json!({"json": json}));
    assert!(d.preview().is_err_and(|e| e.contains("needed")));
    assert!(frames(&mut d, 2) > 0);
}

#[test]
fn the_preview_follows_every_change_to_the_session() {
    let mut d = Designer::new(Session::default(), Box::new(NoDialogs));
    d.open_sample("rentals");
    let before = d.preview().map(|doc| doc.pages.iter().map(|p| p.items.len()).sum::<usize>()).unwrap_or(0);
    d.run("data.loadCsv", serde_json::json!({"csv": "branch,item,days,amount\nLyon,Scaffold tower,12,540\n"}));
    let after = d.preview().map(|doc| doc.pages.iter().map(|p| p.items.len()).sum::<usize>()).unwrap_or(0);
    assert!(after < before, "{before} items, then {after} with one row");
    // A read-only command doesn't render again.
    let revision = d.revision;
    d.run("report.validate", serde_json::json!({}));
    assert_eq!(d.revision, revision);
}

/// Dialogs that answer with files in a folder.
struct Answers(PathBuf);

impl crate::Dialogs for Answers {
    fn open(&self, _: &str, extensions: &[&str]) -> Option<PathBuf> {
        extensions.first().map(|e| self.0.join(format!("in.{e}")))
    }

    fn save(&self, _: &str, extension: &str, _: &str) -> Option<PathBuf> {
        Some(self.0.join(format!("out.{extension}")))
    }
}

#[test]
fn reports_are_saved_opened_and_exported() {
    let dir = std::env::temp_dir().join(format!("reportcraft-designer-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap_or_default();
    let mut d = Designer::new(Session::default(), Box::new(Answers(dir.clone())));
    d.open_sample("rentals");
    let saved = dir.join("rentals.rcr.json");
    d.save_report(&saved);
    assert!(d.status().starts_with("saved"), "{}", d.status());
    // Another report, then the saved one back (the sample's data stays loaded).
    d.run("report.load", serde_json::json!({"json": serde_json::to_string(&reportcraft_model::Report::new("other")).unwrap_or_default()}));
    assert_eq!(d.session().report.title, "other");
    d.open_report(&saved);
    assert_eq!(d.session().report.title, "Rentals by branch");
    for format in ["html", "xlsx", "docx"] {
        let out = dir.join(format!("r.{format}"));
        d.export(format, &out);
        assert!(std::fs::metadata(&out).is_ok_and(|m| m.len() > 100), "{format}: {}", d.status());
    }
    d.open_report(&dir.join("missing.json"));
    assert!(d.status().contains("missing.json"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn elements_are_labelled_by_what_they_show() {
    assert_eq!(canvas::label(&Content::Field { field: "amount".into() }), "[amount]");
    assert_eq!(canvas::label(&Content::Formula { expression: "=[a]*2".into() }), "=[a]*2");
    assert_eq!(canvas::label(&Content::Aggregate { function: reportcraft_model::AggregateFn::Avg, field: "days".into() }), "AVERAGE([days])");
    let mut r = reportcraft_model::Report::new("t");
    r.groups.push(reportcraft_model::Group {
        field: "branch".into(),
        keep_together: false,
        repeat_header: false,
        page_break_before: false,
        header: Default::default(),
        footer: Default::default(),
    });
    assert_eq!(canvas::band_title(&r, "groups[0].header"), "Group 1 header (branch)");
    assert_eq!(canvas::band_title(&r, "page_footer"), "Page footer");
}

#[test]
fn the_preview_fits_the_page_to_the_panel() {
    let doc = reportcraft_layout::Document { title: "t".into(), width_mm: 210.0, height_mm: 297.0, margin_mm: 15.0, pages: Vec::new() };
    assert!((preview::scale_for(&doc, 420.0) - 2.0).abs() < 1e-6);
    let flat = reportcraft_layout::Document { width_mm: 0.0, ..doc };
    assert!((preview::scale_for(&flat, 420.0) - 1.0).abs() < 1e-6);
}

#[test]
fn fields_dropped_moved_resized_and_deleted_are_commands() {
    use crate::canvas::{Event, FieldDrop};
    let mut d = Designer::new(Session::default(), Box::new(NoDialogs));
    d.open_sample("rentals");
    let detail = d.session().report.detail.elements.len();
    d.apply(Event::Drop { band: "detail".into(), x_mm: 150.0, y_mm: 0.5, what: FieldDrop::Field("days".into()) });
    assert_eq!(d.session().report.detail.elements.len(), detail + 1);
    assert_eq!(d.selected, Some(("detail".into(), detail)), "the new element is selected");
    let el = d.session().report.detail.elements.last().cloned().unwrap_or_else(|| FieldDrop::Field(String::new()).element(0.0, 0.0));
    assert_eq!((el.x_mm, &el.content), (150.0, &Content::Field { field: "days".into() }));
    d.apply(Event::Move { band: "detail".into(), index: detail, to: "detail".into(), x_mm: 140.0, y_mm: 1.0 });
    assert_eq!(d.session().report.detail.elements.last().map(|e| (e.x_mm, e.y_mm)), Some((140.0, 1.0)));
    d.apply(Event::Resize { band: "detail".into(), index: detail, width_mm: 20.0, height_mm: 4.5 });
    assert_eq!(d.session().report.detail.elements.last().map(|e| (e.width_mm, e.height_mm)), Some((20.0, 4.5)));
    // Into the report footer: it moves there and stays selected.
    let footer = d.session().report.report_footer.elements.len();
    d.apply(Event::Move { band: "detail".into(), index: detail, to: "report_footer".into(), x_mm: 10.0, y_mm: 0.0 });
    assert_eq!(d.session().report.detail.elements.len(), detail);
    assert_eq!(d.selected, Some(("report_footer".into(), footer)));
    d.delete_selected();
    assert_eq!(d.session().report.report_footer.elements.len(), footer);
    assert!(d.selected.is_none());
    // A move the report can't hold is refused and says why.
    d.apply(Event::Resize { band: "detail".into(), index: 0, width_mm: 20.0, height_mm: 9_000.0 });
    assert!(!d.status().is_empty());
}

#[test]
fn insert_adds_each_kind_to_the_selected_band_and_the_panel_edits_it() {
    let mut d = Designer::new(Session::default(), Box::new(NoDialogs));
    d.open_sample("rentals");
    d.selected_band = Some("report_footer".into());
    let before = d.session().report.report_footer.elements.len();
    for kind in ["Text", "Field", "Formula", "Total", "Page number", "Line", "Box"] {
        d.insert(kind);
    }
    assert_eq!(d.session().report.report_footer.elements.len(), before + 7, "{}", d.status());
    assert!(
        d.session().report.report_footer.elements.iter().any(|e| matches!(&e.content, Content::Formula { expression } if expression == "[branch]"))
    );
    // The panel's working copy follows the selection; a finished edit is one command.
    d.sync_draft();
    let Some((_, crate::Draft::Element(band, index, mut el))) = d.draft.take() else { panic!("an element is selected") };
    el.style.bold = true;
    d.run("report.setElement", serde_json::json!({"band": band, "index": index, "element": el}));
    d.sync_draft();
    assert!(matches!(&d.draft, Some((_, crate::Draft::Element(_, _, e))) if e.style.bold));
    // Nothing selected: the report itself.
    d.selected = None;
    d.selected_band = None;
    d.sync_draft();
    assert!(matches!(&d.draft, Some((_, crate::Draft::Report(r))) if r.title == "Rentals by branch" && r.paper == "custom"));
    assert!(frames(&mut d, 2) > 50, "the panel draws every editor");
}

/// Runs one frame with these pointer events (a 1400 × 900 window, as `frames`).
fn frame_with(ctx: &egui::Context, d: &mut Designer, events: Vec<egui::Event>) {
    let input =
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| d.ui(ui));
    out.textures_delta.clear();
}

fn pointer(ctx: &egui::Context, d: &mut Designer, from: egui::Pos2, to: egui::Pos2) {
    let press = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame_with(ctx, d, vec![egui::Event::PointerMoved(from)]);
    frame_with(ctx, d, vec![press(from, true)]);
    for k in 1..=8 {
        let p = from + (to - from) * (k as f32 / 8.0);
        frame_with(ctx, d, vec![egui::Event::PointerMoved(p)]);
    }
    frame_with(ctx, d, vec![press(to, false)]);
    frame_with(ctx, d, vec![]);
}

#[test]
fn a_field_dragged_with_the_mouse_lands_in_the_band_under_it() {
    let mut d = Designer::new(Session::default(), Box::new(NoDialogs));
    d.open_sample("rentals");
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame_with(&ctx, &mut d, vec![]);
    }
    let detail = d.session().report.detail.elements.len();
    // `days` in the field list, onto the right of the detail band (see the designer screenshot).
    pointer(&ctx, &mut d, egui::pos2(20.0, 94.0), egui::pos2(700.0, 257.0));
    assert_eq!(d.session().report.detail.elements.len(), detail + 1, "{}", d.status());
    let added = d.session().report.detail.elements.last().map(|e| (e.content.clone(), e.x_mm));
    assert!(matches!(&added, Some((Content::Field { field }, x)) if field == "days" && *x > 100.0), "{added:?}");
    // Then the element dragged 20 px to the right: it moves by 20 / 3.5 mm, snapped.
    let x0 = d.session().report.detail.elements.last().map_or(0.0, |e| e.x_mm);
    let at = egui::pos2(178.0 + (x0 as f32 + 2.0) * 3.5, 257.0);
    pointer(&ctx, &mut d, at, at + egui::vec2(21.0, 0.0));
    let x1 = d.session().report.detail.elements.last().map_or(0.0, |e| e.x_mm);
    assert!((x1 - x0 - 6.0).abs() < 0.6, "moved from {x0} to {x1} mm");
}

#[test]
fn undo_and_redo_from_the_menu_and_the_keyboard() {
    let mut d = Designer::new(Session::default(), Box::new(NoDialogs));
    d.open_sample("rentals");
    let elements = d.session().report.detail.elements.len();
    d.insert("Text");
    assert_eq!(d.session().report.detail.elements.len(), elements + 1);
    d.undo();
    assert_eq!(d.session().report.detail.elements.len(), elements);
    d.redo();
    assert_eq!(d.session().report.detail.elements.len(), elements + 1);
    // Ctrl+Z through the window's input.
    let ctx = egui::Context::default();
    frame_with(&ctx, &mut d, vec![]);
    let key = |modifiers| egui::Event::Key { key: egui::Key::Z, physical_key: None, pressed: true, repeat: false, modifiers };
    frame_with(&ctx, &mut d, vec![key(egui::Modifiers::COMMAND)]);
    assert_eq!(d.session().report.detail.elements.len(), elements, "{}", d.status());
    frame_with(&ctx, &mut d, vec![key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT)]);
    assert_eq!(d.session().report.detail.elements.len(), elements + 1);
    // Undo past the start says so.
    for _ in 0..5 {
        d.undo();
    }
    assert!(d.status().contains("nothing to undo"), "{}", d.status());
}

#[test]
fn the_control_channel_runs_commands_as_an_agent() {
    let mut d = Designer::new(Session::default(), Box::new(NoDialogs));
    d.open_sample("rentals");
    let r = d.control("report.setProperties", &serde_json::json!({"title": "Remote"}));
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(d.session().report.title, "Remote");
    assert!(d.preview().is_ok());
    // An agent: no raw database path, no confirmed print; the window's own policy comes back.
    let refused = d.control("data.loadSqlite", &serde_json::json!({"path": "/etc/passwd", "query": "SELECT 1"}));
    assert!(refused.is_err_and(|e| e.contains("saved connections only")));
    assert_eq!(d.session().policy, reportcraft_engine::Policy::PERSON);
    assert!(d.control("nope", &serde_json::json!({})).is_err_and(|e| e.contains("unknown method")));
    let state = d.control("ui.state", &serde_json::json!({})).unwrap_or_default();
    assert_eq!((state["title"].as_str(), state["undo"].as_u64()), (Some("Remote"), Some(2)));
}

#[test]
fn the_preview_panel_keeps_its_width_frame_after_frame() {
    let mut d = Designer::new(Session::default(), Box::new(NoDialogs));
    d.open_sample("rentals");
    let ctx = egui::Context::default();
    let width = |ctx: &egui::Context, d: &mut Designer| {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| d.ui(ui));
        out.textures_delta.clear();
        // The page's width in the preview.
        out.shapes
            .iter()
            .filter_map(|s| match &s.shape {
                // Pages, in the preview on the right (the canvas's bands are white too).
                egui::Shape::Rect(r) if r.fill == egui::Color32::WHITE && r.rect.left() > 900.0 => Some(r.rect.width()),
                _ => None,
            })
            .fold(0.0, f32::max)
    };
    let first = (0..3).map(|_| width(&ctx, &mut d)).last().unwrap_or(0.0);
    let later = (0..60).map(|_| width(&ctx, &mut d)).last().unwrap_or(0.0);
    assert!(first > 300.0 && (first - later).abs() < 1.0, "page {first} px wide, then {later} px");
}

#[test]
fn the_french_catalog_parses_and_translates() {
    static FR: &craft_i18n::Embedded = &[("fr", "", include_str!("../../../i18n/fr.tsv"))];
    let (i18n, problems) = craft_i18n::I18n::new(FR, &["fr"]);
    assert!(problems.is_empty(), "{problems:?}");
    assert!(i18n.check_all().is_empty(), "{:?}", i18n.check_all());
    let fr = i18n.from_code("fr").unwrap_or(craft_i18n::Lang::EN);
    assert_eq!(i18n.t(fr, "New report"), "Nouveau rapport");
    assert_eq!(craft_i18n::fmt(i18n.t(fr, "Group {n} header ({field})"), &[("n", "1"), ("field", "branch")]), "En-tête du groupe 1 (branch)");
    // Without catalogs (the default build), English.
    crate::set_language("fr");
    assert!(crate::language() == "en" || crate::language() == "fr");
    crate::set_language("en");
}

#[test]
fn structure_is_edited_in_one_window_and_parameters_reach_the_preview() {
    let mut d = Designer::new(Session::default(), Box::new(NoDialogs));
    d.open_sample("rentals");
    let mut draft = crate::structure::Draft::of(&d.session().report);
    let sort_keys = draft.sort.len();
    draft.sort.push(reportcraft_model::SortKey { field: "amount".into(), descending: true });
    draft.parameters.push(reportcraft_model::Parameter {
        required: true,
        kind: reportcraft_model::ParamType::Number,
        ..reportcraft_model::Parameter::text("min", "Minimum", None)
    });
    d.structure = Some(draft);
    assert!(frames(&mut d, 2) > 50, "the window draws");
    let params = d.structure.as_ref().map(crate::structure::Draft::params).unwrap_or_default();
    assert!(d.run("report.setStructure", params).is_some(), "{}", d.status());
    assert_eq!(d.session().report.sort.len(), sort_keys + 1);
    // A required parameter with no value: the preview says so; given one, it renders.
    assert!(d.preview().is_err_and(|e| e.contains("min")));
    d.param_values.insert("min".into(), "100".into());
    d.revision += 1;
    assert!(d.preview().is_ok());
    d.check();
    assert!(!d.status().is_empty());
}

#[test]
fn print_connections_and_barcodes_from_the_designer() {
    let session = Session::with_saved_connections(reportcraft_connections_in_memory());
    let mut d = Designer::new(session, Box::new(NoDialogs));
    d.open_sample("rentals");
    // Print: the window opens with the printers; Check only says what would be sent.
    d.open_print();
    let check = d.print.as_ref().map(|p| p.params(false, &d.param_values)).unwrap_or_default();
    assert_eq!(check["confirm"], serde_json::json!(false));
    if reportcraft_has_fonts() {
        let r = d.run("file.print", check);
        assert!(r.is_some_and(|r| r["submitted"] == serde_json::json!(false)), "{}", d.status());
    }
    assert!(frames(&mut d, 2) > 0, "the print window draws");
    d.print = None;
    // Connections: save one, list it, test it, delete it.
    let dir = std::env::temp_dir().join(format!("reportcraft-designer-conn-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_default();
    let db = dir.join("x.db");
    std::fs::write(&db, b"").unwrap_or_default();
    d.open_connections();
    if let Some(c) = d.connections.as_mut() {
        c.name = "local".into();
        c.path = db.display().to_string();
    }
    let params = d.connections.as_ref().map(crate::forms::Connections::params).unwrap_or_default();
    assert!(d.run("data.saveConnection", params).is_some(), "{}", d.status());
    assert_eq!(d.connection_list().len(), 1);
    assert!(d.run("data.testConnection", serde_json::json!({"name": "local"})).is_some(), "{}", d.status());
    assert!(frames(&mut d, 2) > 0, "the connections window draws");
    assert!(d.run("data.deleteConnection", serde_json::json!({"name": "local", "confirm": true})).is_some());
    assert!(d.connection_list().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
    // A barcode, inserted and drawn.
    d.selected_band = Some("detail".into());
    d.insert("Barcode");
    assert!(d.session().report.detail.elements.iter().any(|e| matches!(e.content, Content::Barcode { .. })));
    d.sync_draft();
    assert!(frames(&mut d, 2) > 50);
}

fn reportcraft_connections_in_memory() -> reportcraft_connections::Connections {
    reportcraft_connections::Connections::in_memory()
}

fn reportcraft_has_fonts() -> bool {
    reportcraft_pdf::Fonts::find().is_ok()
}
