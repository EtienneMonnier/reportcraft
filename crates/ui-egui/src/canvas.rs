//! The banded canvas: every band of the report, in print order, at its height, each element in
//! its box with what it shows (`[field]`, `SUM([amount])`, `=formula`…).

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use reportcraft_model::{AggregateFn, Content, Element, Report};

/// Height of the strip naming a band, in pixels.
const LABEL_PX: f32 = 18.0;
const GAP_PX: f32 = 6.0;

/// What an element shows on the canvas.
pub(crate) fn label(content: &Content) -> String {
    match content {
        Content::Text { text } => text.clone(),
        Content::Field { field } => format!("[{field}]"),
        Content::Aggregate { function, field } => {
            let f = match function {
                AggregateFn::Sum => "SUM",
                AggregateFn::Count => "COUNT",
                AggregateFn::Avg => "AVERAGE",
                AggregateFn::Min => "MIN",
                AggregateFn::Max => "MAX",
            };
            format!("{f}([{field}])")
        }
        Content::Parameter { name } => format!("PARAM(\"{name}\")"),
        Content::Formula { expression } => format!("={}", expression.trim_start_matches('=')),
        Content::PageNumber => "page N / M".into(),
        Content::RefreshedAt => "refreshed at".into(),
        Content::PrintedAt => "printed at".into(),
        Content::Line { .. } | Content::Rect { .. } => String::new(),
        Content::Barcode { symbology, field, text, .. } => {
            format!("{symbology:?}: {}", field.as_ref().map_or_else(|| text.clone().unwrap_or_default(), |f| format!("[{f}]")))
        }
        Content::Subreport { dataset, .. } => format!("subreport over `{dataset}`"),
    }
}

/// A band's human name (`groups[0].header` → `Group 1 header (branch)`).
pub(crate) fn band_title(report: &Report, name: &str) -> String {
    let group = |i: &str| i.parse::<usize>().ok().and_then(|i| report.groups.get(i).map(|g| (i + 1, g.field.clone())));
    if let Some(rest) = name.strip_prefix("groups[")
        && let Some((i, part)) = rest.split_once("].")
        && let Some((n, field)) = group(i)
    {
        return format!("Group {n} {part} ({field})");
    }
    match name {
        "report_header" => "Report header",
        "page_header" => "Page header",
        "detail" => "Detail",
        "page_footer" => "Page footer",
        "report_footer" => "Report footer",
        other => other,
    }
    .to_owned()
}

fn colour(c: &str) -> Color32 {
    reportcraft_model::Color::parse(c).map_or(Color32::BLACK, |c| Color32::from_rgb(c.0, c.1, c.2))
}

/// Draws the canvas; returns the element clicked this frame, if any.
pub(crate) fn show(ui: &mut egui::Ui, report: &Report, zoom: f32, selected: Option<&(String, usize)>) -> Option<(String, usize)> {
    let width_mm = report.page.body_width() as f32;
    let bands = report.bands();
    let mut clicked = None;
    egui::ScrollArea::both().show(ui, |ui| {
        let height: f32 = bands.iter().map(|(_, b)| LABEL_PX + b.height_mm as f32 * zoom + GAP_PX).sum();
        let (area, _) = ui.allocate_exact_size(Vec2::new(width_mm * zoom + 2.0, height.max(1.0)), Sense::hover());
        let painter = ui.painter_at(area);
        let mut y = area.top();
        for (name, band) in &bands {
            painter.text(
                Pos2::new(area.left(), y + 2.0),
                Align2::LEFT_TOP,
                band_title(report, name),
                FontId::proportional(12.0),
                ui.visuals().strong_text_color(),
            );
            y += LABEL_PX;
            let band_rect = Rect::from_min_size(Pos2::new(area.left(), y), Vec2::new(width_mm * zoom, band.height_mm as f32 * zoom));
            painter.rect(band_rect, CornerRadius::ZERO, Color32::WHITE, Stroke::new(1.0, Color32::from_gray(170)), StrokeKind::Inside);
            for (i, el) in band.elements.iter().enumerate() {
                let rect = element_rect(band_rect.min, el, zoom);
                let is_selected = selected.is_some_and(|(b, k)| b == name && *k == i);
                draw_element(&painter, el, rect, zoom, is_selected);
                let id = ui.make_persistent_id(("element", name.as_str(), i));
                if ui.interact(rect.expand(2.0), id, Sense::click()).clicked() {
                    clicked = Some((name.clone(), i));
                }
            }
            y += band_rect.height() + GAP_PX;
        }
    });
    clicked
}

/// An element's box on the canvas.
pub(crate) fn element_rect(band_top_left: Pos2, el: &Element, zoom: f32) -> Rect {
    Rect::from_min_size(
        band_top_left + Vec2::new(el.x_mm as f32 * zoom, el.y_mm as f32 * zoom),
        Vec2::new((el.width_mm as f32 * zoom).max(1.0), (el.height_mm as f32 * zoom).max(1.0)),
    )
}

fn draw_element(painter: &egui::Painter, el: &Element, rect: Rect, zoom: f32, selected: bool) {
    let outline = if selected { Stroke::new(2.0, Color32::from_rgb(0x1e, 0x6f, 0xd9)) } else { Stroke::new(1.0, Color32::from_gray(200)) };
    match &el.content {
        Content::Line { thickness_mm, color } => {
            let stroke = Stroke::new((*thickness_mm as f32 * zoom).max(1.0), colour(color));
            painter
                .line_segment([rect.left_top(), Pos2::new(rect.left() + el.width_mm as f32 * zoom, rect.top() + el.height_mm as f32 * zoom)], stroke);
        }
        Content::Rect { border_mm, border_color, fill } => {
            let fill = fill.as_deref().map_or(Color32::TRANSPARENT, colour);
            painter.rect(rect, CornerRadius::ZERO, fill, Stroke::new((*border_mm as f32 * zoom).max(1.0), colour(border_color)), StrokeKind::Middle);
        }
        content => {
            let bg = el.style.background.map_or(Color32::from_rgb(0xf4, 0xf7, 0xfb), |c| Color32::from_rgb(c.0, c.1, c.2));
            painter.rect_filled(rect, CornerRadius::ZERO, bg);
            let text = label(content);
            let align = match el.style.align {
                reportcraft_model::Align::Left => Align2::LEFT_CENTER,
                reportcraft_model::Align::Center => Align2::CENTER_CENTER,
                reportcraft_model::Align::Right => Align2::RIGHT_CENTER,
            };
            let at = match el.style.align {
                reportcraft_model::Align::Left => rect.left_center() + Vec2::new(2.0, 0.0),
                reportcraft_model::Align::Center => rect.center(),
                reportcraft_model::Align::Right => rect.right_center() - Vec2::new(2.0, 0.0),
            };
            let size = (el.style.size_pt as f32 * 0.3528 * zoom).clamp(6.0, 48.0);
            let color = el.style.color.map_or(Color32::from_gray(40), |c| Color32::from_rgb(c.0, c.1, c.2));
            painter.with_clip_rect(rect).text(at, align, text, FontId::proportional(size), color);
        }
    }
    painter.rect_stroke(rect, CornerRadius::ZERO, outline, StrokeKind::Outside);
}
