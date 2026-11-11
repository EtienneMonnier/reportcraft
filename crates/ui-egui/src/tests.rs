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
