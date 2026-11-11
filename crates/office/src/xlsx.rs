//! A laid-out report to an XLSX workbook: one sheet, a row per line of the report, a column per
//! left edge, numbers and dates kept as typed cells with their number format.

use std::collections::BTreeMap;

use reportcraft_core::{Cancel, Error, Result, Value};
use reportcraft_layout::{Document, Item};
use reportcraft_model::{Align, Format};
use rust_xlsxwriter::{DocProperties, Format as XFormat, FormatAlign, Workbook, Worksheet};

use crate::{Output, lines, mm_to_points, skipped_warning};

/// Excel's widest column, in pixels (255 characters).
const MAX_COLUMN_PX: f64 = 1_790.0;
/// Excel's tallest row, in points.
const MAX_ROW_PT: f64 = 409.0;
/// Excel's last row and column.
const MAX_ROWS: usize = 1_048_576;
const MAX_COLUMNS: usize = 16_384;

/// Writes `doc` (best laid out continuously: `reportcraft_layout::render_continuous`) as XLSX.
pub fn to_xlsx(doc: &Document, cancel: &Cancel) -> Result<Output> {
    let fail = |e: rust_xlsxwriter::XlsxError| Error::Invalid(format!("XLSX: {e}"));
    let mut workbook = Workbook::new();
    // A fixed creation date: the same report gives the same file.
    let epoch = chrono::NaiveDate::from_ymd_opt(1980, 1, 1).unwrap_or_default();
    workbook.set_properties(&DocProperties::new().set_title(&doc.title).set_creation_datetime(&epoch));
    let sheet = workbook.add_worksheet();
    sheet.set_name(sheet_name(&doc.title)).map_err(fail)?;

    let (rows, skipped) = lines(doc);
    let columns = column_edges(rows.iter().flatten().map(|it| it.x_mm));
    if rows.len() > MAX_ROWS || columns.len() > MAX_COLUMNS {
        return Err(Error::TooLarge("the report has more rows or columns than a spreadsheet holds".into()));
    }
    // Column widths: up to the next column's edge, the last one as wide as its widest item.
    let right = rows.iter().flatten().map(|it| it.x_mm + it.width_mm).fold(0.0, f64::max);
    let edges: Vec<f64> = columns.keys().map(|k| *k as f64 / 10.0).collect();
    for (c, x) in edges.iter().enumerate() {
        let next = edges.get(c + 1).copied().unwrap_or(right);
        let px = ((next - x) * 96.0 / 25.4).clamp(8.0, MAX_COLUMN_PX);
        sheet.set_column_width_pixels(u16::try_from(c).unwrap_or(u16::MAX), px.round() as u32).map_err(fail)?;
    }
    for (r, line) in rows.iter().enumerate() {
        if r % 1_024 == 0 {
            cancel.check("the XLSX export")?;
        }
        let row = u32::try_from(r).map_err(|_| Error::TooLarge("too many rows".into()))?;
        let height = line.iter().map(|it| mm_to_points(it.height_mm).max(it.style.size_pt * 1.3)).fold(0.0, f64::max);
        sheet.set_row_height(row, height.clamp(1.0, MAX_ROW_PT)).map_err(fail)?;
        // Items sharing a column on one line are joined, keeping the first one's style.
        let mut cells: BTreeMap<u16, Vec<&Item>> = BTreeMap::new();
        for it in line {
            let col = columns.get(&key(it.x_mm)).copied().unwrap_or(0);
            cells.entry(col).or_default().push(it);
        }
        for (col, items) in cells {
            write_cell(sheet, row, col, &items).map_err(fail)?;
        }
    }
    let bytes = workbook.save_to_buffer().map_err(fail)?;
    Ok(Output { bytes, warnings: skipped_warning(skipped, "XLSX") })
}

/// A position in tenths of a millimetre, the precision columns are told apart at.
fn key(x_mm: f64) -> i64 {
    (x_mm * 10.0).round() as i64
}

/// Every distinct left edge, in order, with its column number.
fn column_edges(xs: impl Iterator<Item = f64>) -> BTreeMap<i64, u16> {
    let mut edges: BTreeMap<i64, u16> = xs.map(|x| (key(x), 0)).collect();
    for (i, v) in edges.values_mut().enumerate() {
        *v = u16::try_from(i).unwrap_or(u16::MAX);
    }
    edges
}

/// A sheet name Excel accepts: at most 31 characters, none of `[]:*?/\`.
fn sheet_name(title: &str) -> String {
    let name: String = title.chars().filter(|c| !"[]:*?/\\'".contains(*c)).take(31).collect();
    let name = name.trim().to_owned();
    if name.is_empty() { "Report".to_owned() } else { name }
}

fn write_cell(sheet: &mut Worksheet, row: u32, col: u16, items: &[&Item]) -> std::result::Result<(), rust_xlsxwriter::XlsxError> {
    let Some(first) = items.first() else { return Ok(()) };
    let mut format = XFormat::new().set_font_size(first.style.size_pt);
    if first.style.bold {
        format = format.set_bold();
    }
    let rgb = |c: reportcraft_model::Color| rust_xlsxwriter::Color::RGB((u32::from(c.0) << 16) | (u32::from(c.1) << 8) | u32::from(c.2));
    if let Some(c) = first.style.color {
        format = format.set_font_color(rgb(c));
    }
    if let Some(b) = first.style.background {
        format = format.set_background_color(rgb(b));
    }
    format = match first.style.align {
        Align::Left => format,
        Align::Center => format.set_align(FormatAlign::Center),
        Align::Right => format.set_align(FormatAlign::Right),
    };
    if items.len() > 1 {
        let text: Vec<&str> = items.iter().map(|it| it.text.as_str()).filter(|t| !t.is_empty()).collect();
        sheet.write_string_with_format(row, col, text.join(" "), &format)?;
        return Ok(());
    }
    let cell = first.cell.as_ref();
    let num_format = cell.and_then(|c| c.format.as_ref()).and_then(excel_format);
    if let Some(code) = &num_format {
        format = format.set_num_format(code);
    }
    match cell.map(|c| (&c.value, c.format.as_ref())) {
        Some((Value::Int(i), _)) => sheet.write_number_with_format(row, col, *i as f64, &format)?,
        Some((Value::Number(n), _)) if n.is_finite() => sheet.write_number_with_format(row, col, *n, &format)?,
        Some((Value::Bool(b), _)) => sheet.write_boolean_with_format(row, col, *b, &format)?,
        Some((Value::Text(t), Some(Format::Date { .. }))) if num_format.is_some() => match date(t) {
            Some(dt) => sheet.write_datetime_with_format(row, col, dt, &format)?,
            None => sheet.write_string_with_format(row, col, &first.text, &format)?,
        },
        _ => sheet.write_string_with_format(row, col, &first.text, &format)?,
    };
    Ok(())
}

/// A date or date-time written as text by a data source (ISO 8601, SQL, RFC 3339).
fn date(text: &str) -> Option<chrono::NaiveDateTime> {
    let t = text.trim();
    if let Ok(d) = chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        return d.and_hms_opt(0, 0, 0);
    }
    for p in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(t, p) {
            return Some(dt);
        }
    }
    chrono::DateTime::parse_from_rfc3339(t).ok().map(|dt| dt.naive_local())
}

/// An element format as an Excel number format code, which Excel then shows with the reader's
/// own separators. `None` when the format can't be expressed (the cell is then written as text).
pub(crate) fn excel_format(format: &Format) -> Option<String> {
    let decimals = |d: u8| if d == 0 { String::new() } else { format!(".{}", "0".repeat(usize::from(d.min(10)))) };
    Some(match format {
        Format::Number { decimals: d, grouping } => format!("{}{}", if *grouping { "#,##0" } else { "0" }, decimals(*d)),
        Format::Currency { symbol, decimals: d, symbol_after } => {
            let symbol: String = symbol.chars().filter(|c| *c != '"' && *c != '\\').collect();
            if *symbol_after { format!("#,##0{} \"{symbol}\"", decimals(*d)) } else { format!("\"{symbol}\"#,##0{}", decimals(*d)) }
        }
        Format::Percent { decimals: d } => format!("0{}%", decimals(*d)),
        Format::Date { pattern } => excel_date(pattern.as_deref().unwrap_or("%Y-%m-%d"))?,
    })
}

/// A `strftime` pattern as an Excel date code (`%d/%m/%Y` → `dd/mm/yyyy`).
fn excel_date(pattern: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            if " /-:.,()".contains(c) {
                out.push(c);
            } else {
                out.push('"');
                out.push(c);
                out.push('"');
            }
            continue;
        }
        let unpadded = chars.peek() == Some(&'-');
        if unpadded {
            chars.next();
        }
        let code = match (chars.next()?, unpadded) {
            ('d', false) => "dd",
            ('d' | 'e', _) => "d",
            ('m', false) => "mm",
            ('m', true) => "m",
            ('Y', _) => "yyyy",
            ('y', _) => "yy",
            ('H' | 'I', false) => "hh",
            ('H' | 'I', true) => "h",
            ('M', _) => "mm",
            ('S', _) => "ss",
            ('B', _) => "mmmm",
            ('b' | 'h', _) => "mmm",
            ('A', _) => "dddd",
            ('a', _) => "ddd",
            ('p', _) => "AM/PM",
            ('F', _) => "yyyy-mm-dd",
            ('T', _) => "hh:mm:ss",
            ('R', _) => "hh:mm",
            ('D', _) => "mm/dd/yy",
            ('%', _) => "\"%\"",
            _ => return None,
        };
        out.push_str(code);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_become_excel_codes() {
        let f = |format: Format| excel_format(&format).unwrap();
        assert_eq!(f(Format::Number { decimals: 2, grouping: true }), "#,##0.00");
        assert_eq!(f(Format::Number { decimals: 0, grouping: false }), "0");
        assert_eq!(f(Format::Currency { symbol: "€".into(), decimals: 2, symbol_after: true }), "#,##0.00 \"€\"");
        assert_eq!(f(Format::Currency { symbol: "$\"".into(), decimals: 0, symbol_after: false }), "\"$\"#,##0");
        assert_eq!(f(Format::Percent { decimals: 1 }), "0.0%");
        assert_eq!(f(Format::Date { pattern: Some("%d/%m/%Y %H:%M".into()) }), "dd/mm/yyyy hh:mm");
        assert_eq!(f(Format::Date { pattern: Some("%-d %B %Y".into()) }), "d mmmm yyyy");
        assert_eq!(f(Format::Date { pattern: Some("Le %d".into()) }), "\"L\"\"e\" dd");
        assert_eq!(f(Format::Date { pattern: None }), "yyyy-mm-dd");
        assert!(excel_format(&Format::Date { pattern: Some("%j".into()) }).is_none(), "day of year: no Excel code");
        assert!(excel_format(&Format::Date { pattern: Some("%".into()) }).is_none());
    }

    #[test]
    fn sheet_names_are_what_excel_accepts() {
        assert_eq!(sheet_name("Rentals [Q4] / 2026: all?"), "Rentals Q4  2026 all");
        assert_eq!(sheet_name("  "), "Report");
        assert_eq!(sheet_name(&"x".repeat(40)).len(), 31);
    }
}
