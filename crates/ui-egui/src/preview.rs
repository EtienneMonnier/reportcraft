//! The live preview: the rendered pages, scaled to the panel's width, drawn from the same
//! `Document` the HTML, PDF and Office writers read.

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use reportcraft_layout::{Document, Item, Shape};
use reportcraft_model::Align;

const MM_PER_PT: f32 = 25.4 / 72.0;
const PAGE_GAP_PX: f32 = 12.0;

fn colour(c: &str) -> Color32 {
    reportcraft_model::Color::parse(c).map_or(Color32::BLACK, |c| Color32::from_rgb(c.0, c.1, c.2))
}

/// The scale (pixels per millimetre) that fits a page in `width` pixels.
pub(crate) fn scale_for(doc: &Document, width: f32) -> f32 {
    if doc.width_mm > 0.0 { (width / doc.width_mm as f32).max(0.1) } else { 1.0 }
}

pub(crate) fn show(ui: &mut egui::Ui, doc: &Document) {
    ui.label(format!("{} page(s)", doc.pages.len()));
    egui::ScrollArea::vertical().show(ui, |ui| {
        let scale = scale_for(doc, ui.available_width() - 8.0);
        let (w, h) = (doc.width_mm as f32 * scale, doc.height_mm as f32 * scale);
        for page in &doc.pages {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect(rect, CornerRadius::ZERO, Color32::WHITE, Stroke::new(1.0, Color32::from_gray(160)), StrokeKind::Outside);
            for it in &page.items {
                draw(&painter, rect.min, it, scale);
            }
            ui.add_space(PAGE_GAP_PX);
        }
    });
}

fn item_rect(origin: Pos2, it: &Item, scale: f32) -> Rect {
    Rect::from_min_size(
        origin + Vec2::new(it.x_mm as f32 * scale, it.y_mm as f32 * scale),
        Vec2::new(it.width_mm as f32 * scale, it.height_mm as f32 * scale),
    )
}

fn draw(painter: &egui::Painter, origin: Pos2, it: &Item, scale: f32) {
    let rect = item_rect(origin, it, scale);
    if let Some(b) = it.style.background
        && it.shape.is_none()
    {
        painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgb(b.0, b.1, b.2));
    }
    match &it.shape {
        Some(Shape::Line { thickness_mm, color }) => {
            let end = origin + Vec2::new((it.x_mm + it.width_mm) as f32 * scale, (it.y_mm + it.height_mm) as f32 * scale);
            painter.line_segment([rect.min, end], Stroke::new((*thickness_mm as f32 * scale).max(0.5), colour(color)));
        }
        Some(Shape::Rect { border_mm, border_color, fill }) => {
            let fill = fill.as_deref().map_or(Color32::TRANSPARENT, colour);
            let stroke = if *border_mm > 0.0 { Stroke::new((*border_mm as f32 * scale).max(0.5), colour(border_color)) } else { Stroke::NONE };
            painter.rect(rect, CornerRadius::ZERO, fill, stroke, StrokeKind::Middle);
        }
        Some(Shape::Barcode { columns, rows, quiet, modules }) => barcode(painter, rect, *columns, *rows, *quiet, modules),
        None => text(painter, rect, it, scale),
    }
}

fn text(painter: &egui::Painter, rect: Rect, it: &Item, scale: f32) {
    if it.text.is_empty() {
        return;
    }
    let size = (it.style.size_pt as f32 * MM_PER_PT * scale).max(1.0);
    let colour = it.style.color.map_or(Color32::BLACK, |c| Color32::from_rgb(c.0, c.1, c.2));
    // The text's line box is 1.2 × its size, from the item's top, as in the other writers.
    let top = rect.top() + size * 0.1;
    let (at, align) = match it.style.align {
        Align::Left => (Pos2::new(rect.left(), top), Align2::LEFT_TOP),
        Align::Center => (Pos2::new(rect.center().x, top), Align2::CENTER_TOP),
        Align::Right => (Pos2::new(rect.right(), top), Align2::RIGHT_TOP),
    };
    let font = FontId::proportional(size);
    let clipped = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    clipped.text(at, align, &it.text, font.clone(), colour);
    // No bold face in the default fonts: a second pass a hair to the right reads as bold.
    if it.style.bold {
        clipped.text(at + Vec2::new((size * 0.04).max(0.4), 0.0), align, &it.text, font, colour);
    }
}

/// A barcode as the PDF writer draws it: linear codes stretched to the box, matrix codes square.
fn barcode(painter: &egui::Painter, rect: Rect, columns: usize, rows: usize, quiet: usize, modules: &str) {
    if columns == 0 || rows == 0 || modules.len() != columns.saturating_mul(rows) {
        return;
    }
    let span = (columns + 2 * quiet) as f32;
    let (w, h, sx, sy) = if rows == 1 {
        (rect.width(), rect.height(), rect.width() / span, rect.height())
    } else {
        let side = rect.width().min(rect.height());
        let unit = side / ((rows + 2 * quiet) as f32).max(span);
        (side, side, unit, unit)
    };
    painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(w, h)), CornerRadius::ZERO, Color32::WHITE);
    let ox = rect.left() + quiet as f32 * sx;
    let oy = rect.top() + if rows == 1 { 0.0 } else { quiet as f32 * sy };
    for (y, row) in modules.as_bytes().chunks(columns).enumerate() {
        for (x, m) in row.iter().enumerate() {
            if *m == b'1' {
                let r = Rect::from_min_size(Pos2::new(ox + x as f32 * sx, oy + y as f32 * sy), Vec2::new(sx, sy));
                painter.rect_filled(r, CornerRadius::ZERO, Color32::BLACK);
            }
        }
    }
}
