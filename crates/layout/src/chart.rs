//! Charts drawn as items: bars and markers as rectangles, lines as segments, pie wedges as
//! polygons, labels as text. Every writer that draws shapes and text draws a chart.

use reportcraft_core::{RowSet, Value};
use reportcraft_model::{Align, ChartKind, Content, Element, LocaleStyle, MAX_CHART_CATEGORIES, TextStyle};

use crate::{Item, Shape, aggregate, format, row_of};

/// The colours series and wedges take, in turn (chosen for contrast on white and with each other).
pub(crate) const PALETTE: &[&str] = &["#3b6ea8", "#e08a2e", "#4c9a5b", "#c44e52", "#8172b2", "#937860", "#d17fb5", "#7f7f7f"];

const LABEL_MM: f64 = 4.0;
const TITLE_MM: f64 = 6.0;
const AXIS: &str = "#666666";

/// The categories in the order they first appear, each with its total.
pub(crate) fn series(el: &Element, rows: &RowSet, summary: &[usize]) -> Vec<(String, Value)> {
    let Content::Chart { category, value, function, .. } = &el.content else { return Vec::new() };
    let (Some(c), Some(v)) = (rows.column_index(category), rows.column_index(value)) else { return Vec::new() };
    let mut order: Vec<String> = Vec::new();
    let mut members: std::collections::HashMap<String, Vec<usize>> = std::collections::HashMap::new();
    for &r in summary {
        let key = rows.value(row_of(rows, r), c).to_string();
        let entry = members.entry(key.clone()).or_default();
        if entry.is_empty() {
            if order.len() >= MAX_CHART_CATEGORIES {
                continue;
            }
            order.push(key);
        }
        entry.push(r);
    }
    order
        .into_iter()
        .map(|k| {
            let rs = members.get(&k).map_or(&[][..], Vec::as_slice);
            let total = aggregate(*function, rs.iter().map(|&r| rows.value(row_of(rows, r), v)));
            (k, total)
        })
        .collect()
}

/// A text item in the box `(x, y, w, h)`, styled `(size, alignment, bold)`.
fn text((x, y, w, h): (f64, f64, f64, f64), (size_pt, align, bold): (f64, Align, bool), t: String) -> Item {
    Item {
        x_mm: x,
        y_mm: y,
        width_mm: w.max(0.0),
        height_mm: h,
        style: TextStyle { size_pt, bold, align, ..TextStyle::default() },
        text: t,
        shape: None,
        cell: None,
    }
}

fn shape(x: f64, y: f64, w: f64, h: f64, shape: Shape) -> Item {
    Item { x_mm: x, y_mm: y, width_mm: w, height_mm: h, style: TextStyle::default(), text: String::new(), shape: Some(shape), cell: None }
}

fn filled(x: f64, y: f64, w: f64, h: f64, colour: &str) -> Item {
    shape(x, y, w, h, Shape::Rect { border_mm: 0.0, border_color: colour.into(), fill: Some(colour.into()) })
}

/// The chart of `el` at `origin` (its box's top-left corner on the page).
pub(crate) fn draw(el: &Element, rows: &RowSet, summary: &[usize], origin: (f64, f64), locale: &LocaleStyle) -> Vec<Item> {
    let Content::Chart { chart: kind, title, .. } = &el.content else { return Vec::new() };
    let (x0, mut top) = origin;
    let (w, bottom) = (el.width_mm, origin.1 + el.height_mm);
    let mut items = Vec::new();
    if let Some(t) = title.as_ref().filter(|t| !t.is_empty()) {
        items.push(text((x0, top, w, TITLE_MM - 1.0), (9.0, Align::Center, true), t.clone()));
        top += TITLE_MM;
    }
    let data = series(el, rows, summary);
    let values: Vec<(String, f64, String)> =
        data.iter().filter_map(|(k, v)| v.as_f64().map(|n| (k.clone(), n, format::write(v, el.format.as_ref(), locale)))).collect();
    match kind {
        ChartKind::Pie => pie(&mut items, &values, (x0, top, w, bottom - top)),
        ChartKind::Bar | ChartKind::Line => axes(&mut items, &values, *kind, (x0, top, w, bottom - top)),
    }
    items
}

/// Bars or a line over a zero baseline, a value above each point and a category below.
fn axes(items: &mut Vec<Item>, values: &[(String, f64, String)], kind: ChartKind, (x0, top, w, h): (f64, f64, f64, f64)) {
    let plot_top = top + LABEL_MM;
    let plot_bottom = top + h - LABEL_MM;
    let plot_h = plot_bottom - plot_top;
    if plot_h <= 1.0 || w <= 2.0 {
        return;
    }
    let lo = values.iter().map(|v| v.1).fold(0.0, f64::min);
    let hi = values.iter().map(|v| v.1).fold(0.0, f64::max);
    let span = if hi > lo { hi - lo } else { 1.0 };
    let ypos = |v: f64| plot_bottom - (v - lo) / span * plot_h;
    let base = ypos(0.0);
    items.push(shape(x0, base, w, 0.0, Shape::Line { thickness_mm: 0.2, color: AXIS.into() }));
    let n = values.len().max(1) as f64;
    let slot = w / n;
    let mut previous: Option<(f64, f64)> = None;
    for (i, (label, v, shown)) in values.iter().enumerate() {
        let cx = x0 + slot * (i as f64 + 0.5);
        let y = ypos(*v);
        match kind {
            ChartKind::Bar => {
                let bw = slot * 0.7;
                items.push(filled(cx - bw / 2.0, y.min(base), bw, (y - base).abs(), PALETTE[0]));
            }
            _ => {
                if let Some((px, py)) = previous {
                    items.push(shape(px, py, cx - px, y - py, Shape::Line { thickness_mm: 0.5, color: PALETTE[0].into() }));
                }
                items.push(filled(cx - 0.6, y - 0.6, 1.2, 1.2, PALETTE[0]));
                previous = Some((cx, y));
            }
        }
        let value_y = if *v >= 0.0 { y - LABEL_MM + 0.5 } else { y + 0.5 };
        items.push(text((x0 + slot * i as f64, value_y, slot, LABEL_MM - 0.5), (6.5, Align::Center, false), shown.clone()));
        items.push(text((x0 + slot * i as f64, plot_bottom + 0.5, slot, LABEL_MM - 0.5), (7.0, Align::Center, false), label.clone()));
    }
}

/// Wedges from the top, clockwise, and a legend with each category's share.
fn pie(items: &mut Vec<Item>, values: &[(String, f64, String)], (x0, top, w, h): (f64, f64, f64, f64)) {
    let positive: Vec<&(String, f64, String)> = values.iter().filter(|v| v.1 > 0.0).collect();
    let total: f64 = positive.iter().map(|v| v.1).sum();
    let r = (h.min(w * 0.55) / 2.0 - 0.5).max(0.0);
    if total <= 0.0 || r <= 0.5 {
        return;
    }
    let (cx, cy) = (x0 + r + 0.5, top + h / 2.0);
    let mut angle = -std::f64::consts::FRAC_PI_2;
    let legend_x = cx + r + 4.0;
    for (i, (label, v, _)) in positive.iter().enumerate() {
        let colour = PALETTE[i % PALETTE.len()];
        let sweep = v / total * std::f64::consts::TAU;
        // The arc in steps of at most 5 degrees.
        let steps = ((sweep.to_degrees() / 5.0).ceil() as usize).max(1);
        let mut points = vec![[cx, cy]];
        for s in 0..=steps {
            let a = angle + sweep * s as f64 / steps as f64;
            points.push([cx + r * a.cos(), cy + r * a.sin()]);
        }
        angle += sweep;
        items.push(shape(cx - r, cy - r, 2.0 * r, 2.0 * r, Shape::Polygon { points, fill: colour.into() }));
        let row_y = top + i as f64 * 4.5;
        if row_y + 4.0 <= top + h {
            items.push(filled(legend_x, row_y + 0.8, 2.6, 2.6, colour));
            let share = format!("{label}  {:.1} %", v / total * 100.0);
            items.push(text((legend_x + 3.6, row_y, (x0 + w - legend_x - 3.6).max(0.0), 4.0), (7.0, Align::Left, false), share));
        }
    }
}
