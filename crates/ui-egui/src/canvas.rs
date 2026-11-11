//! The banded canvas: every band of the report, in print order, at its height, each element in
//! its box with what it shows (`[field]`, `SUM([amount])`, `=formula`…). Elements are selected
//! by a click, moved by dragging (into another band too) and resized by the handle at their
//! bottom-right corner; fields dragged from the field list are dropped into a band. The canvas
//! only reports what happened ([`Event`]); the designer turns it into engine commands.

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use reportcraft_model::{AggregateFn, Content, Element, Report};

/// Height of the strip naming a band, in pixels.
const LABEL_PX: f32 = 18.0;
const GAP_PX: f32 = 6.0;
const HANDLE_PX: f32 = 7.0;
/// Positions and sizes snap to this many millimetres.
pub(crate) const SNAP_MM: f64 = 0.5;

/// Something dragged from the field list.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FieldDrop {
    /// A column of the data, or a named formula (both are fields).
    Field(String),
    Parameter(String),
}

impl FieldDrop {
    /// The element it becomes when dropped at `x`, `y` (millimetres in the band).
    pub(crate) fn element(&self, x_mm: f64, y_mm: f64) -> Element {
        let content = match self {
            FieldDrop::Field(f) => Content::Field { field: f.clone() },
            FieldDrop::Parameter(p) => Content::Parameter { name: p.clone() },
        };
        Element {
            x_mm,
            y_mm,
            width_mm: 35.0,
            height_mm: 5.0,
            style: Default::default(),
            content,
            format: None,
            visible_if: None,
            color_formula: None,
            background_formula: None,
        }
    }
}

/// What the person did on the canvas this frame.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Event {
    Select(String, usize),
    SelectBand(String),
    /// An element dragged to `x`, `y` (millimetres) in `to` (its own band or another).
    Move {
        band: String,
        index: usize,
        to: String,
        x_mm: f64,
        y_mm: f64,
    },
    Resize {
        band: String,
        index: usize,
        width_mm: f64,
        height_mm: f64,
    },
    Drop {
        band: String,
        x_mm: f64,
        y_mm: f64,
        what: FieldDrop,
    },
}

/// A drag in progress: the element and how far it has gone, in pixels.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Drag {
    band: String,
    index: usize,
    resize: bool,
    offset: Vec2,
}

/// The canvas's own state between frames.
#[derive(Debug, Default)]
pub(crate) struct State {
    drag: Option<Drag>,
}

/// What an element shows on the canvas.
pub(crate) fn label(content: &Content) -> String {
    match content {
        Content::Text { text } => text.clone(),
        Content::Field { field } => format!("[{field}]"),
        Content::Aggregate { function, field } => format!("{}([{field}])", aggregate_name(*function)),
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
        Content::Chart { chart, category, value, function, .. } => {
            let kind = match chart {
                reportcraft_model::ChartKind::Bar => tl!("Bar chart"),
                reportcraft_model::ChartKind::Line => tl!("Line chart"),
                reportcraft_model::ChartKind::Pie => tl!("Pie chart"),
            };
            format!("{kind}: {}([{value}]) / [{category}]", aggregate_name(*function))
        }
    }
}

pub(crate) fn aggregate_name(f: AggregateFn) -> &'static str {
    match f {
        AggregateFn::Sum => "SUM",
        AggregateFn::Count => "COUNT",
        AggregateFn::Avg => "AVERAGE",
        AggregateFn::Min => "MIN",
        AggregateFn::Max => "MAX",
    }
}

/// A band's human name (`groups[0].header` → `Group 1 header (branch)`).
pub(crate) fn band_title(report: &Report, name: &str) -> String {
    let group = |i: &str| i.parse::<usize>().ok().and_then(|i| report.groups.get(i).map(|g| (i + 1, g.field.clone())));
    if let Some(rest) = name.strip_prefix("groups[")
        && let Some((i, part)) = rest.split_once("].")
        && let Some((n, field)) = group(i)
    {
        let template = if part == "header" { "Group {n} header ({field})" } else { "Group {n} footer ({field})" };
        return tf!(template, n = n, field = field);
    }
    match name {
        "report_header" => tl!("Report header"),
        "page_header" => tl!("Page header"),
        "detail" => tl!("Detail"),
        "page_footer" => tl!("Page footer"),
        "report_footer" => tl!("Report footer"),
        other => other,
    }
    .to_owned()
}

fn colour(c: &str) -> Color32 {
    reportcraft_model::Color::parse(c).map_or(Color32::BLACK, |c| Color32::from_rgb(c.0, c.1, c.2))
}

/// `v` millimetres snapped to the grid, never negative.
pub(crate) fn snap(v: f64) -> f64 {
    ((v / SNAP_MM).round() * SNAP_MM).max(0.0)
}

/// Draws the canvas; returns what the person did this frame.
pub(crate) fn show(ui: &mut egui::Ui, report: &Report, zoom: f32, selected: Option<&(String, usize)>, state: &mut State) -> Vec<Event> {
    let width_mm = report.page.body_width() as f32;
    let bands = report.bands();
    let mut events = Vec::new();
    egui::ScrollArea::both().show(ui, |ui| {
        let height: f32 = bands.iter().map(|(_, b)| LABEL_PX + (b.height_mm as f32 * zoom).max(2.0) + GAP_PX).sum();
        let (area, _) = ui.allocate_exact_size(Vec2::new(width_mm * zoom + 2.0, height.max(1.0)), Sense::hover());
        let painter = ui.painter_at(area);
        // Band rectangles first: drops and moves land in the band under the pointer.
        let mut rects: Vec<(String, Rect)> = Vec::with_capacity(bands.len());
        let mut y = area.top();
        for (name, band) in &bands {
            let label_rect = Rect::from_min_size(Pos2::new(area.left(), y), Vec2::new(width_mm * zoom, LABEL_PX));
            if ui.interact(label_rect, ui.make_persistent_id(("band", name.as_str())), Sense::click()).clicked() {
                events.push(Event::SelectBand(name.clone()));
            }
            painter.text(
                label_rect.left_top() + Vec2::new(0.0, 2.0),
                Align2::LEFT_TOP,
                band_title(report, name),
                FontId::proportional(12.0),
                ui.visuals().strong_text_color(),
            );
            y += LABEL_PX;
            let band_rect = Rect::from_min_size(Pos2::new(area.left(), y), Vec2::new(width_mm * zoom, (band.height_mm as f32 * zoom).max(2.0)));
            painter.rect(band_rect, CornerRadius::ZERO, Color32::WHITE, Stroke::new(1.0, Color32::from_gray(170)), StrokeKind::Inside);
            rects.push((name.clone(), band_rect));
            y += band_rect.height() + GAP_PX;
        }
        let band_at = |p: Pos2| rects.iter().find(|(_, r)| r.expand2(Vec2::new(0.0, GAP_PX / 2.0)).contains(p)).map(|(n, r)| (n.clone(), *r));
        for ((name, band), (_, band_rect)) in bands.iter().zip(&rects) {
            let frame = Frame { painter: &painter, band_rect: *band_rect, zoom, selected, band_at: &band_at };
            for (i, el) in band.elements.iter().enumerate() {
                element(ui, &frame, (name, i, el), state, &mut events);
            }
        }
        // A field dropped from the field list.
        if egui::DragAndDrop::has_payload_of_type::<FieldDrop>(ui.ctx()) {
            let pointer = ui.ctx().pointer_interact_pos();
            if let Some((_, r)) = pointer.and_then(band_at) {
                painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(2.0, Color32::from_rgb(0x1e, 0x6f, 0xd9)), StrokeKind::Inside);
            }
            if ui.input(|i| i.pointer.any_released())
                && let Some(p) = pointer
                && let Some((band, r)) = band_at(p)
                && let Some(what) = egui::DragAndDrop::take_payload::<FieldDrop>(ui.ctx())
            {
                events.push(Event::Drop {
                    band,
                    x_mm: snap(f64::from((p.x - r.left()) / zoom)),
                    y_mm: snap(f64::from((p.y - r.top()) / zoom)),
                    what: (*what).clone(),
                });
            }
        }
    });
    events
}

/// What every element of a band is drawn with.
struct Frame<'a> {
    painter: &'a egui::Painter,
    band_rect: Rect,
    zoom: f32,
    selected: Option<&'a (String, usize)>,
    band_at: &'a dyn Fn(Pos2) -> Option<(String, Rect)>,
}

/// One element: drawn where it is (or where it is being dragged), with its click, drag and
/// resize interactions.
fn element(ui: &egui::Ui, f: &Frame<'_>, (name, i, el): (&String, usize, &Element), state: &mut State, events: &mut Vec<Event>) {
    let Frame { painter, band_rect, zoom, selected, band_at } = *f;
    let home = element_rect(band_rect.min, el, zoom);
    let mut rect = home;
    if let Some(d) = state.drag.as_ref().filter(|d| &d.band == name && d.index == i) {
        if d.resize {
            rect.max += d.offset;
        } else {
            rect = rect.translate(d.offset);
        }
    }
    let is_selected = selected.is_some_and(|(b, k)| b == name && *k == i);
    draw_element(painter, el, rect, zoom, is_selected);
    let response = ui.interact(home.expand(2.0), ui.make_persistent_id(("element", name.as_str(), i)), Sense::click_and_drag());
    if response.clicked() || response.drag_started() {
        events.push(Event::Select(name.clone(), i));
    }
    if response.dragged() {
        let d = state.drag.get_or_insert_with(|| Drag { band: name.clone(), index: i, resize: false, offset: Vec2::ZERO });
        d.offset = dragged_by(ui, d.offset + response.drag_delta());
    }
    if response.drag_stopped()
        && let Some(d) = state.drag.take()
    {
        let moved = home.translate(d.offset);
        let (to, target) = response.interact_pointer_pos().and_then(band_at).unwrap_or((name.clone(), band_rect));
        events.push(Event::Move {
            band: name.clone(),
            index: i,
            to,
            x_mm: snap(f64::from((moved.left() - target.left()) / zoom)),
            y_mm: snap(f64::from((moved.top() - target.top()) / zoom)),
        });
    }
    if is_selected {
        let handle = Rect::from_center_size(rect.right_bottom(), Vec2::splat(HANDLE_PX));
        painter.rect_filled(handle, CornerRadius::ZERO, Color32::from_rgb(0x1e, 0x6f, 0xd9));
        let r = ui.interact(
            Rect::from_center_size(home.right_bottom(), Vec2::splat(HANDLE_PX)),
            ui.make_persistent_id(("resize", name.as_str(), i)),
            Sense::drag(),
        );
        if r.dragged() {
            let d = state.drag.get_or_insert_with(|| Drag { band: name.clone(), index: i, resize: true, offset: Vec2::ZERO });
            d.offset = dragged_by(ui, d.offset + r.drag_delta());
        }
        if r.drag_stopped()
            && let Some(d) = state.drag.take()
        {
            let size = home.size() + d.offset;
            events.push(Event::Resize {
                band: name.clone(),
                index: i,
                width_mm: snap(f64::from(size.x / zoom)).max(SNAP_MM),
                height_mm: snap(f64::from(size.y / zoom)),
            });
        }
    }
}

/// How far the pointer has gone since the button was pressed (the drag's first pixels, before
/// egui calls it a drag, included), or `fallback` when the input doesn't say.
fn dragged_by(ui: &egui::Ui, fallback: Vec2) -> Vec2 {
    ui.input(|i| match (i.pointer.press_origin(), i.pointer.interact_pos()) {
        (Some(from), Some(to)) => to - from,
        _ => fallback,
    })
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
            let (at, align) = match el.style.align {
                reportcraft_model::Align::Left => (rect.left_center() + Vec2::new(2.0, 0.0), Align2::LEFT_CENTER),
                reportcraft_model::Align::Center => (rect.center(), Align2::CENTER_CENTER),
                reportcraft_model::Align::Right => (rect.right_center() - Vec2::new(2.0, 0.0), Align2::RIGHT_CENTER),
            };
            let size = (el.style.size_pt as f32 * 0.3528 * zoom).clamp(6.0, 48.0);
            let color = el.style.color.map_or(Color32::from_gray(40), |c| Color32::from_rgb(c.0, c.1, c.2));
            painter.with_clip_rect(rect).text(at, align, text, FontId::proportional(size), color);
        }
    }
    painter.rect_stroke(rect, CornerRadius::ZERO, outline, StrokeKind::Outside);
}
