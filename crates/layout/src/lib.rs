//! ReportCraft layout (L3): lays a report and its rows out as pages of positioned text,
//! toolkit-free. Writers (HTML here; PDF next, through craft-libs' craft-pdf) read `Document`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod format;
mod html;

use std::cmp::Ordering;
use std::collections::BTreeMap;

use reportcraft_core::{Error, Result, RowSet, Value, limits};
use reportcraft_model::{AggregateFn, Band, Content, Report, TextStyle};
use serde::{Deserialize, Serialize};

pub use format::write as format_value;
pub use html::to_html;

/// A laid-out report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub title: String,
    pub width_mm: f64,
    pub height_mm: f64,
    pub pages: Vec<Page>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Page {
    pub items: Vec<Item>,
}

/// One piece of text, positioned in millimetres from the page's top-left corner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub x_mm: f64,
    pub y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    pub style: TextStyle,
    pub text: String,
}

/// Parameter values for one render, by parameter name.
pub type Params = BTreeMap<String, String>;

/// Lays `report` out over `rows`.
pub fn layout(report: &Report, rows: &RowSet, params: &Params) -> Result<Document> {
    report.validate()?;
    let group_cols = report
        .groups
        .iter()
        .map(|g| rows.column_index(&g.field).ok_or_else(|| Error::NotFound(format!("group field `{}`", g.field))))
        .collect::<Result<Vec<_>>>()?;
    let order = sorted_rows(report, rows, &group_cols)?;
    let mut w = Writer::new(report, rows, params);
    // Page one: the report header comes first, then the page header.
    w.start_page(false)?;
    w.band(&report.report_header, Scope { rows: &order, current: order.first().copied() })?;
    w.page_header();
    let mut open_from = 0;
    for (pos, &row) in order.iter().enumerate() {
        if pos > 0 {
            let prev = order.get(pos - 1).copied().unwrap_or(row);
            let level = group_cols.iter().position(|&c| rows.value(row_of(rows, prev), c) != rows.value(row_of(rows, row), c));
            if let Some(level) = level {
                w.close_groups(&order, pos, level, &group_cols)?;
                open_from = level;
            } else {
                open_from = group_cols.len();
            }
        }
        for level in open_from..group_cols.len() {
            let end = group_end(rows, &order, pos, level, &group_cols);
            let header = report.groups.get(level).map(|g| &g.header);
            if let (Some(header), Some(scope)) = (header, order.get(pos..end)) {
                w.band(header, Scope { rows: scope, current: Some(row) })?;
            }
        }
        w.band(&report.detail, Scope { rows: std::slice::from_ref(&row), current: Some(row) })?;
    }
    if !order.is_empty() {
        w.close_groups(&order, order.len(), 0, &group_cols)?;
    }
    w.band(&report.report_footer, Scope { rows: &order, current: order.last().copied() })?;
    w.finish_page()?;
    Ok(w.into_document())
}

fn row_of(rows: &RowSet, i: usize) -> &[reportcraft_core::Value] {
    rows.rows.get(i).map_or(&[], Vec::as_slice)
}

/// Row indices sorted by the group fields, then the report's sort keys (stable).
fn sorted_rows(report: &Report, rows: &RowSet, group_cols: &[usize]) -> Result<Vec<usize>> {
    let mut keys: Vec<(usize, bool)> = group_cols.iter().map(|&c| (c, false)).collect();
    for s in &report.sort {
        let c = rows.column_index(&s.field).ok_or_else(|| Error::NotFound(format!("sort field `{}`", s.field)))?;
        keys.push((c, s.descending));
    }
    let mut order: Vec<usize> = (0..rows.rows.len().min(limits::MAX_ROWS)).collect();
    order.sort_by(|&a, &b| {
        keys.iter()
            .map(|&(c, desc)| {
                let o = rows.value(row_of(rows, a), c).sort_cmp(rows.value(row_of(rows, b), c));
                if desc { o.reverse() } else { o }
            })
            .find(|o| *o != Ordering::Equal)
            .unwrap_or(Ordering::Equal)
    });
    Ok(order)
}

/// One past the last position of the group at `level` that starts at `start`.
fn group_end(rows: &RowSet, order: &[usize], start: usize, level: usize, group_cols: &[usize]) -> usize {
    let first = order.get(start).map_or(&[][..], |&r| row_of(rows, r));
    let same = |r: usize| group_cols.iter().take(level + 1).all(|&c| rows.value(row_of(rows, r), c) == rows.value(first, c));
    order.iter().skip(start).position(|&r| !same(r)).map_or(order.len(), |n| start + n)
}

/// The rows a band summarises, and the row its fields read.
#[derive(Clone, Copy)]
struct Scope<'a> {
    rows: &'a [usize],
    current: Option<usize>,
}

struct Writer<'a> {
    report: &'a Report,
    rows: &'a RowSet,
    params: &'a Params,
    locale: reportcraft_model::LocaleStyle,
    pages: Vec<Page>,
    page: Page,
    y: f64,
    /// Items that show the page number, by page.
    page_number_items: Vec<(usize, usize)>,
}

impl<'a> Writer<'a> {
    fn new(report: &'a Report, rows: &'a RowSet, params: &'a Params) -> Self {
        let locale = reportcraft_model::locale_or_default(&report.locale);
        Self { report, rows, params, locale, pages: Vec::new(), page: Page::default(), y: 0.0, page_number_items: Vec::new() }
    }

    fn body_bottom(&self) -> f64 {
        self.report.page.height_mm - self.report.page.margin_mm - self.report.page_footer.height_mm
    }

    fn start_page(&mut self, with_header: bool) -> Result<()> {
        if self.pages.len() >= limits::MAX_PAGES {
            return Err(Error::TooLarge(format!("more than {} pages", limits::MAX_PAGES)));
        }
        self.page = Page::default();
        self.y = self.report.page.margin_mm;
        if with_header {
            self.page_header();
        }
        Ok(())
    }

    fn page_header(&mut self) {
        self.draw(&self.report.page_header, Scope { rows: &[], current: None });
        self.y += self.report.page_header.height_mm;
    }

    fn finish_page(&mut self) -> Result<()> {
        self.y = self.body_bottom();
        self.draw(&self.report.page_footer, Scope { rows: &[], current: None });
        self.pages.push(std::mem::take(&mut self.page));
        Ok(())
    }

    fn band(&mut self, band: &Band, scope: Scope<'_>) -> Result<()> {
        if band.is_empty() {
            return Ok(());
        }
        if self.y + band.height_mm > self.body_bottom() + 1e-9 {
            self.finish_page()?;
            self.start_page(true)?;
        }
        self.draw(band, scope);
        self.y += band.height_mm;
        Ok(())
    }

    /// Closes groups from the innermost down to `level`, at the row before `pos`.
    fn close_groups(&mut self, order: &[usize], pos: usize, level: usize, group_cols: &[usize]) -> Result<()> {
        let last = pos.checked_sub(1).and_then(|p| order.get(p)).copied();
        for l in (level..group_cols.len()).rev() {
            let start = self.group_start(order, pos, l, group_cols);
            let footer = self.report.groups.get(l).map(|g| &g.footer);
            if let (Some(footer), Some(scope)) = (footer, order.get(start..pos)) {
                self.band(footer, Scope { rows: scope, current: last })?;
            }
        }
        Ok(())
    }

    /// Where the group at `level` that ends just before `pos` started.
    fn group_start(&self, order: &[usize], pos: usize, level: usize, group_cols: &[usize]) -> usize {
        let Some(&last) = pos.checked_sub(1).and_then(|p| order.get(p)) else { return pos };
        let rows = self.rows;
        let same = |r: usize| group_cols.iter().take(level + 1).all(|&c| rows.value(row_of(rows, r), c) == rows.value(row_of(rows, last), c));
        let before = order.get(..pos).unwrap_or(&[]);
        before.iter().rev().position(|&r| !same(r)).map_or(0, |n| pos - n)
    }

    fn draw(&mut self, band: &Band, scope: Scope<'_>) {
        let left = self.report.page.margin_mm;
        for el in &band.elements {
            if matches!(el.content, Content::PageNumber) {
                self.page_number_items.push((self.pages.len(), self.page.items.len()));
            }
            let text = self.eval(&el.content, el.format.as_ref(), scope);
            self.page.items.push(Item {
                x_mm: left + el.x_mm,
                y_mm: self.y + el.y_mm,
                width_mm: el.width_mm,
                height_mm: el.height_mm,
                style: el.style,
                text,
            });
        }
    }

    fn eval(&self, content: &Content, format: Option<&reportcraft_model::Format>, scope: Scope<'_>) -> String {
        let column = |field: &str| self.rows.column_index(field);
        match content {
            Content::Text { text } => text.clone(),
            Content::Field { field } => match (column(field), scope.current) {
                (Some(c), Some(r)) => format::write(self.rows.value(row_of(self.rows, r), c), format, &self.locale),
                _ => String::new(),
            },
            Content::Aggregate { function, field } => match column(field) {
                Some(c) => {
                    let total = aggregate(*function, scope.rows.iter().map(|&r| self.rows.value(row_of(self.rows, r), c)));
                    // A count is a count: a currency or date format on the field doesn't apply to it.
                    let format = if *function == AggregateFn::Count { None } else { format };
                    format::write(&total, format, &self.locale)
                }
                None => String::new(),
            },
            Content::Parameter { name } => {
                let raw = self
                    .params
                    .get(name)
                    .cloned()
                    .or_else(|| self.report.parameters.iter().find(|p| &p.name == name).and_then(|p| p.default.clone()));
                raw.map(|r| format::write(&Value::Text(r), format, &self.locale)).unwrap_or_default()
            }
            Content::PageNumber => String::new(),
        }
    }

    fn into_document(mut self) -> Document {
        let total = self.pages.len();
        for (page, item) in std::mem::take(&mut self.page_number_items) {
            if let Some(it) = self.pages.get_mut(page).and_then(|p| p.items.get_mut(item)) {
                it.text = format!("{} / {total}", page + 1);
            }
        }
        Document { title: self.report.title.clone(), width_mm: self.report.page.width_mm, height_mm: self.report.page.height_mm, pages: self.pages }
    }
}

fn aggregate<'v>(function: AggregateFn, values: impl Iterator<Item = &'v Value>) -> Value {
    let present: Vec<&Value> = values.filter(|v| !v.is_null()).collect();
    if function == AggregateFn::Count {
        return Value::Int(i64::try_from(present.len()).unwrap_or(i64::MAX));
    }
    let nums: Vec<f64> = present.iter().filter_map(|v| v.as_f64()).collect();
    if nums.is_empty() {
        return Value::Null;
    }
    let n = match function {
        AggregateFn::Sum => nums.iter().sum(),
        AggregateFn::Avg => nums.iter().sum::<f64>() / nums.len() as f64,
        AggregateFn::Min => nums.iter().copied().fold(f64::INFINITY, f64::min),
        AggregateFn::Max => nums.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        AggregateFn::Count => 0.0,
    };
    if n.is_finite() { Value::Number(n) } else { Value::Null }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reportcraft_core::{Column, FieldType};
    use reportcraft_model::{Element, Group};

    fn el(content: Content) -> Element {
        Element { x_mm: 0.0, y_mm: 0.0, width_mm: 50.0, height_mm: 5.0, style: TextStyle::default(), content, format: None }
    }

    fn rows() -> RowSet {
        let col = |n: &str, t| Column { name: n.into(), field_type: t };
        RowSet {
            columns: vec![col("branch", FieldType::Text), col("amount", FieldType::Number)],
            rows: vec![
                vec![Value::Text("Lyon".into()), Value::Number(10.0)],
                vec![Value::Text("Paris".into()), Value::Number(5.0)],
                vec![Value::Text("Lyon".into()), Value::Number(2.5)],
            ],
        }
    }

    fn texts(doc: &Document) -> Vec<String> {
        doc.pages.iter().flat_map(|p| p.items.iter().map(|i| i.text.clone())).filter(|t| !t.is_empty()).collect()
    }

    #[test]
    fn groups_sort_and_total() {
        let mut r = Report::new("t");
        r.groups.push(Group {
            field: "branch".into(),
            header: Band { height_mm: 6.0, elements: vec![el(Content::Field { field: "branch".into() })] },
            footer: Band { height_mm: 6.0, elements: vec![el(Content::Aggregate { function: AggregateFn::Sum, field: "amount".into() })] },
        });
        r.detail = Band { height_mm: 5.0, elements: vec![el(Content::Field { field: "amount".into() })] };
        r.report_footer = Band { height_mm: 6.0, elements: vec![el(Content::Aggregate { function: AggregateFn::Sum, field: "amount".into() })] };
        let doc = layout(&r, &rows(), &Params::new()).unwrap();
        assert_eq!(texts(&doc), ["Lyon", "10", "2.5", "12.5", "Paris", "5", "5", "17.5"]);
    }

    #[test]
    fn breaks_pages_and_numbers_them() {
        let mut r = Report::new("t");
        r.page.height_mm = 60.0;
        r.page.margin_mm = 10.0;
        r.page_footer = Band { height_mm: 5.0, elements: vec![el(Content::PageNumber)] };
        r.detail = Band { height_mm: 15.0, elements: vec![el(Content::Field { field: "branch".into() })] };
        let doc = layout(&r, &rows(), &Params::new()).unwrap();
        assert_eq!(doc.pages.len(), 2);
        assert!(texts(&doc).contains(&"2 / 2".to_owned()));
    }

    #[test]
    fn unknown_group_field_is_an_error_and_empty_data_still_renders() {
        let mut r = Report::new("t");
        r.groups.push(Group { field: "nope".into(), header: Band::default(), footer: Band::default() });
        assert!(matches!(layout(&r, &rows(), &Params::new()), Err(Error::NotFound(_))));
        let doc = layout(&Report::new("t"), &RowSet::default(), &Params::new()).unwrap();
        assert_eq!(doc.pages.len(), 1);
    }
}
