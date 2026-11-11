//! Cross-tabs drawn as items: a header line with the column values, one line per row value,
//! each cell the total of its rows, totals on the right and at the bottom, a frame and rules.
//! Cells keep their typed value, so a spreadsheet gets numbers.

use std::collections::HashMap;

use reportcraft_core::{RowSet, Value};
use reportcraft_model::{AggregateFn, Align, Content, Element, LocaleStyle, MAX_CROSSTAB_COLUMNS, TextStyle};

use crate::{Cell, Item, Shape, aggregate, format, row_of};

const MM_PER_PT: f64 = 25.4 / 72.0;
/// Most lines a cross-tab draws, whatever its height.
const MAX_LINES: usize = 500;

/// The distinct values of column `c` over `rows`, ascending.
fn keys(set: &RowSet, rows: &[usize], c: usize) -> Vec<Value> {
    let mut v: Vec<Value> = rows.iter().map(|&r| set.value(row_of(set, r), c).clone()).collect();
    v.sort_by(Value::sort_cmp);
    v.dedup();
    v
}

/// The cross-tab of `el` at `origin` (its box's top-left corner on the page).
pub(crate) fn draw(el: &Element, set: &RowSet, summary: &[usize], origin: (f64, f64), locale: &LocaleStyle) -> Vec<Item> {
    let Content::Crosstab { rows, columns, value, function, totals } = &el.content else { return Vec::new() };
    let (Some(rc), Some(cc), Some(vc)) = (set.column_index(rows), set.column_index(columns), set.column_index(value)) else { return Vec::new() };
    let row_keys = keys(set, summary, rc);
    let mut col_keys = keys(set, summary, cc);
    col_keys.truncate(MAX_CROSSTAB_COLUMNS);
    // The rows of each cell, line and column.
    let index = |v: &Value| v.to_string();
    let mut cells: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for &r in summary {
        let row = row_of(set, r);
        cells.entry((index(set.value(row, rc)), index(set.value(row, cc)))).or_default().push(r);
    }
    let total_of = |members: &mut dyn Iterator<Item = usize>| {
        aggregate(*function, members.map(|r| set.value(row_of(set, r), vc)).collect::<Vec<_>>().into_iter())
    };
    let line_mm = el.style.size_pt * 1.2 * MM_PER_PT + 1.0;
    let (x0, y0) = origin;
    let first = (el.width_mm * 0.3).max(15.0).min(el.width_mm);
    let n = col_keys.len() + usize::from(*totals);
    let col_w = if n == 0 { 0.0 } else { (el.width_mm - first) / n as f64 };
    let fits = ((el.height_mm / line_mm).floor() as usize).min(MAX_LINES);
    let body = fits.saturating_sub(1 + usize::from(*totals));
    let shown = if row_keys.len() > body { body.saturating_sub(1) } else { row_keys.len() };
    let style = |bold: bool, align: Align| TextStyle { bold, align, ..el.style };
    let mut items = Vec::new();
    let mut text = |x: f64, line: usize, w: f64, t: String, st: TextStyle, cell: Option<Cell>| {
        items.push(Item {
            x_mm: x + 0.8,
            y_mm: y0 + line as f64 * line_mm + 0.5,
            width_mm: (w - 1.6).max(0.0),
            height_mm: line_mm - 1.0,
            style: st,
            text: t,
            shape: None,
            cell,
        });
    };
    let number = |v: Value, bold: bool| {
        let shown = format::write(&v, if *function == AggregateFn::Count { None } else { el.format.as_ref() }, locale);
        let keep = matches!(v, Value::Int(_) | Value::Number(_));
        (shown, style(bold, Align::Right), keep.then(|| Cell { value: v, format: el.format.clone() }))
    };
    // Header.
    for (i, k) in col_keys.iter().enumerate() {
        text(x0 + first + i as f64 * col_w, 0, col_w, k.to_string(), style(true, Align::Right), None);
    }
    if *totals {
        text(x0 + first + col_keys.len() as f64 * col_w, 0, col_w, "Total".into(), style(true, Align::Right), None);
    }
    // Body.
    for (line, rk) in row_keys.iter().take(shown).enumerate() {
        let line = line + 1;
        text(x0, line, first, rk.to_string(), style(false, Align::Left), None);
        for (i, ck) in col_keys.iter().enumerate() {
            if let Some(members) = cells.get(&(index(rk), index(ck))) {
                let (t, st, cell) = number(total_of(&mut members.iter().copied()), false);
                text(x0 + first + i as f64 * col_w, line, col_w, t, st, cell);
            }
        }
        if *totals {
            let mut all = col_keys.iter().filter_map(|ck| cells.get(&(index(rk), index(ck)))).flatten().copied();
            let (t, st, cell) = number(total_of(&mut all), true);
            text(x0 + first + col_keys.len() as f64 * col_w, line, col_w, t, st, cell);
        }
    }
    let mut last = shown + 1;
    if shown < row_keys.len() {
        text(x0, last, el.width_mm, format!("… {} more", row_keys.len() - shown), style(false, Align::Left), None);
        last += 1;
    }
    // Totals at the bottom.
    let totals_line = last;
    if *totals {
        text(x0, last, first, "Total".into(), style(true, Align::Left), None);
        for (i, ck) in col_keys.iter().enumerate() {
            let mut column = row_keys.iter().filter_map(|rk| cells.get(&(index(rk), index(ck)))).flatten().copied();
            let (t, st, cell) = number(total_of(&mut column), true);
            text(x0 + first + i as f64 * col_w, last, col_w, t, st, cell);
        }
        let (t, st, cell) = number(total_of(&mut summary.iter().copied()), true);
        text(x0 + first + col_keys.len() as f64 * col_w, last, col_w, t, st, cell);
        last += 1;
    }
    // Frame and rules.
    let rule = |x: f64, y: f64, w: f64, h: f64| Item {
        x_mm: x,
        y_mm: y,
        width_mm: w,
        height_mm: h,
        style: TextStyle::default(),
        text: String::new(),
        shape: Some(Shape::Line { thickness_mm: 0.2, color: "#888888".into() }),
        cell: None,
    };
    let height = last as f64 * line_mm;
    items.push(Item {
        x_mm: x0,
        y_mm: y0,
        width_mm: el.width_mm,
        height_mm: height,
        style: TextStyle::default(),
        text: String::new(),
        shape: Some(Shape::Rect { border_mm: 0.2, border_color: "#888888".into(), fill: None }),
        cell: None,
    });
    items.push(rule(x0, y0 + line_mm, el.width_mm, 0.0));
    items.push(rule(x0 + first, y0, 0.0, height));
    if *totals {
        items.push(rule(x0, y0 + totals_line as f64 * line_mm, el.width_mm, 0.0));
        items.push(rule(x0 + first + col_keys.len() as f64 * col_w, y0, 0.0, height));
    }
    items
}
