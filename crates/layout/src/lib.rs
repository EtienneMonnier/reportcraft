//! ReportCraft layout (L3): lays a report and its rows out as pages of positioned text,
//! toolkit-free. Writers (HTML here; PDF next, through craft-libs' craft-pdf) read `Document`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod format;
mod formula;
mod html;

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use reportcraft_core::{Cancel, Error, Result, RowSet, Value, limits};
use reportcraft_model::{AggregateFn, Band, Content, Element, Report, SubreportLink, TextStyle};
use serde::{Deserialize, Serialize};

pub use format::write as format_value;
pub use html::to_html;
pub use reportcraft_model::color_is_valid;

/// A laid-out report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub title: String,
    pub width_mm: f64,
    pub height_mm: f64,
    /// The page margin the layout used (items are already placed inside it).
    #[serde(default)]
    pub margin_mm: f64,
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
    /// A line or rectangle over the item's box instead of text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<Shape>,
    /// The number, boolean or date behind the text, with its format, for writers that keep
    /// values typed (a spreadsheet cell). `None` for fixed text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell: Option<Cell>,
}

/// A typed value and the format its text was written with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<reportcraft_model::Format>,
}

/// A drawn shape. Colours are validated `#rgb`/`#rrggbb` strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    /// From the box's top-left corner to its bottom-right corner.
    Line {
        thickness_mm: f64,
        color: String,
    },
    Rect {
        border_mm: f64,
        border_color: String,
        fill: Option<String>,
    },
    /// A barcode drawn over the box: `rows` × `columns` modules, row-major, `1` dark and `0`
    /// light, to be surrounded by `quiet` light modules. A linear barcode has one row, stretched
    /// to the box's height; a matrix code is square, as large as the box allows.
    Barcode {
        columns: usize,
        rows: usize,
        quiet: usize,
        modules: String,
    },
}

/// The render value holding when the data was read (RFC 3339), for `Content::RefreshedAt`.
pub const REFRESHED_AT: &str = "@refreshed_at";
/// The render value holding when the report was rendered (RFC 3339), for `Content::PrintedAt`.
pub const PRINTED_AT: &str = "@printed_at";

/// Height of the text under a linear barcode.
const BARCODE_TEXT_MM: f64 = 3.5;

/// Rows laid out between two looks at the cancel flag.
const CANCEL_EVERY: usize = 256;

/// Parameter values for one render, by parameter name.
pub type Params = BTreeMap<String, String>;

/// Named datasets that subreports read.
pub type Datasets = BTreeMap<String, Arc<RowSet>>;

/// Lays `report` out over `rows`.
pub fn layout(report: &Report, rows: &RowSet, params: &Params) -> Result<Document> {
    layout_with_cancel(report, rows, params, &Cancel::default())
}

/// `layout`, stopping with an error once `cancel` is raised or past its deadline.
pub fn layout_with_cancel(report: &Report, rows: &RowSet, params: &Params, cancel: &Cancel) -> Result<Document> {
    render(report, rows, &Datasets::new(), params, cancel)
}

/// `layout_with_cancel`, with the datasets the report's subreports read.
pub fn render(report: &Report, rows: &RowSet, datasets: &Datasets, params: &Params, cancel: &Cancel) -> Result<Document> {
    report.validate()?;
    let ctx = Context { datasets, params, cancel, depth: 0 };
    let computed = formula::with_formulas(report, rows, params, cancel)?;
    Ok(run(report, computed.as_ref().unwrap_or(rows), &ctx)?.into_document())
}

/// `render`, on one endless page as wide as the report's body, for writers that don't paginate
/// (a spreadsheet): the page header once at the top, no page footer, one column, no page breaks.
/// Items are placed from the page's corner (no margin).
pub fn render_continuous(report: &Report, rows: &RowSet, datasets: &Datasets, params: &Params, cancel: &Cancel) -> Result<Document> {
    report.validate()?;
    let page = reportcraft_model::PageSetup {
        paper: None,
        orientation: reportcraft_model::Orientation::Portrait,
        width_mm: report.page.body_width(),
        height_mm: SUBREPORT_PAGE_MM,
        margin_mm: 0.0,
        columns: 1,
        column_gap_mm: 0.0,
        column_order: reportcraft_model::ColumnOrder::Down,
    };
    let groups = report
        .groups
        .iter()
        .map(|g| reportcraft_model::Group { keep_together: false, page_break_before: false, repeat_header: false, ..g.clone() })
        .collect();
    let endless = Report { page, groups, page_footer: Band::default(), ..report.clone() };
    let ctx = Context { datasets, params, cancel, depth: 0 };
    let computed = formula::with_formulas(&endless, rows, params, cancel)?;
    let mut doc = run(&endless, computed.as_ref().unwrap_or(rows), &ctx)?.into_document();
    // Nothing goes below the last item.
    doc.height_mm = doc.pages.iter().flat_map(|p| &p.items).map(|i| i.y_mm + i.height_mm).fold(0.0, f64::max);
    Ok(doc)
}

/// The pages `spec` names, in document order: `1-3,5`, `8-` (to the end), `-2` (from the start),
/// spaces allowed. Page numbers start at 1; pages past the end are ignored, a page named twice is
/// kept once. An empty selection or a malformed range is an error.
pub fn select_pages(doc: &Document, spec: &str) -> Result<Document> {
    let count = doc.pages.len();
    let bad = |part: &str| Error::Invalid(format!("pages: `{part}` is not a page or a range like 2-5"));
    let mut keep = vec![false; count];
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let number = |t: &str| t.trim().parse::<usize>().ok().filter(|n| *n >= 1).ok_or_else(|| bad(part));
        let (from, to) = match part.split_once('-') {
            Some((a, b)) => {
                let from = if a.trim().is_empty() { 1 } else { number(a)? };
                // An open end runs to the last page, or names none when it starts past it.
                (from, if b.trim().is_empty() { count.max(from) } else { number(b)? })
            }
            None => (number(part)?, number(part)?),
        };
        if from > to {
            return Err(Error::Invalid(format!("pages: `{part}` ends before it starts")));
        }
        for page in from..=to.min(count) {
            if let Some(k) = keep.get_mut(page - 1) {
                *k = true;
            }
        }
    }
    let pages: Vec<Page> = doc.pages.iter().zip(&keep).filter(|(_, k)| **k).map(|(p, _)| p.clone()).collect();
    if pages.is_empty() {
        return Err(Error::Invalid(format!("pages: `{spec}` names none of the {count} page(s)")));
    }
    Ok(Document { pages, ..doc.clone() })
}

/// The values of `expression` for the first `limit` rows of `rows` (in data order), with the
/// report's named formulas computed and every row summarised: what a formula shows before it is
/// placed in a band.
pub fn evaluate_formula(report: &Report, rows: &RowSet, params: &Params, expression: &str, limit: usize) -> Result<Vec<Value>> {
    let expr = reportcraft_expr::parse(expression).map_err(|e| Error::Invalid(format!("formula: {e}")))?;
    let computed = formula::with_formulas(report, rows, params, &Cancel::default())?;
    let rows = computed.as_ref().unwrap_or(rows);
    let all: Vec<usize> = (0..rows.rows.len()).collect();
    let memo = formula::Memo::default();
    let now = formula::now(params);
    let at = |current: Option<usize>| {
        let env = formula::BandEnv { report, rows, current, summary: &all, params, now, memo: &memo, locale: formula::locale_of(report) };
        formula::to_value(reportcraft_expr::evaluate(&expr, &env))
    };
    if all.is_empty() {
        return Ok(vec![at(None)]);
    }
    Ok(all.iter().take(limit).map(|&r| at(Some(r))).collect())
}

/// What a report and its subreports share while being laid out.
#[derive(Clone, Copy)]
struct Context<'a> {
    datasets: &'a Datasets,
    params: &'a Params,
    cancel: &'a Cancel,
    /// 0 for the main report, 1 for its subreports…
    depth: usize,
}

/// Lays a (validated) report out and hands back the writer with every page.
fn run<'a>(report: &'a Report, rows: &'a RowSet, ctx: &Context<'a>) -> Result<Writer<'a>> {
    let cancel = ctx.cancel;
    let group_cols = report
        .groups
        .iter()
        .map(|g| rows.column_index(&g.field).ok_or_else(|| Error::NotFound(format!("group field `{}`", g.field))))
        .collect::<Result<Vec<_>>>()?;
    let order = sorted_rows(report, rows, &group_cols)?;
    let mut w = Writer::new(report, rows, ctx);
    // Page one: the report header comes first, then the page header.
    w.start_page(false, &order)?;
    w.full_band(&report.report_header, Scope::new(&order, order.first().copied()), &order)?;
    w.page_header();
    let mut open_from = 0;
    for (pos, &row) in order.iter().enumerate() {
        if pos % CANCEL_EVERY == 0 {
            cancel.check("the render")?;
        }
        if pos > 0 {
            let prev = order.get(pos - 1).copied().unwrap_or(row);
            let level = group_cols.iter().position(|&c| rows.value(row_of(rows, prev), c) != rows.value(row_of(rows, row), c));
            if let Some(level) = level {
                w.close_groups(&order, pos, level)?;
                open_from = level;
            } else {
                open_from = group_cols.len();
            }
        }
        for level in open_from..group_cols.len() {
            let end = group_end(rows, &order, pos, level, &group_cols);
            let Some(group) = report.groups.get(level) else { continue };
            if group.page_break_before && !w.fresh {
                w.new_page(&order)?;
            } else if group.keep_together && !w.fresh {
                let height = group_height(report, rows, &order, pos, end, level, &group_cols);
                if height > w.body_bottom() - w.y && height <= w.body_bottom() - w.top_after_headers() {
                    w.new_page(&order)?;
                }
            }
            w.open.push(OpenGroup { level, start: pos, end });
            if let Some(scope) = order.get(pos..end) {
                // What must follow a header kept with it: the inner headers and one detail row.
                let following = if group.header.keep_with_next {
                    report.groups.iter().skip(level + 1).map(|g| g.header.height_mm).sum::<f64>() + report.detail.height_mm
                } else {
                    0.0
                };
                w.band_in_keeping(&group.header, Scope::new(scope, Some(row)), &order, following)?;
            }
        }
        let group = w.open.last().copied().and_then(|g| order.get(g.start..g.end)).unwrap_or(&order);
        let scope = Scope { group: Some(group), ..Scope::new(std::slice::from_ref(&row), Some(row)) };
        w.detail(&report.detail, scope, &order)?;
    }
    if !order.is_empty() {
        w.close_groups(&order, order.len(), 0)?;
    }
    w.full_band(&report.report_footer, Scope::new(&order, order.last().copied()), &order)?;
    w.finish_page()?;
    Ok(w)
}

/// Height of the endless page a subreport is laid out on before it is cut into blocks.
const SUBREPORT_PAGE_MM: f64 = 1e9;

/// A stretch of laid-out content that moves as one: a band, or part of a subreport. Its items
/// run from `item` to the next block's `item`.
#[derive(Debug, Clone, Copy)]
struct Mark {
    item: usize,
    top: f64,
    height: f64,
}

/// A subreport's content cut at its band boundaries, items relative to each block's top.
struct Block {
    height: f64,
    items: Vec<Item>,
}

/// Lays a subreport out over its rows on an endless page as wide as its element, and cuts it
/// into the blocks the parent places one after another.
fn subreport_blocks(child: &Report, rows: &RowSet, ctx: &Context<'_>) -> Result<Vec<Block>> {
    let computed = formula::with_formulas(child, rows, ctx.params, ctx.cancel)?;
    let w = run(child, computed.as_ref().unwrap_or(rows), ctx)?;
    let mut items: Vec<Item> = w.pages.into_iter().flat_map(|p| p.items).collect();
    let marks = w.marks;
    let mut blocks = Vec::with_capacity(marks.len());
    for m in marks.iter().rev() {
        let mut part = items.split_off(m.item.min(items.len()));
        for it in &mut part {
            it.y_mm -= m.top;
        }
        blocks.push(Block { height: m.height, items: part });
    }
    blocks.reverse();
    Ok(blocks)
}

/// The report a subreport element lays out: its own bands on an endless page as wide as the
/// element, without page header or footer.
fn subreport_report(child: &Report, width_mm: f64) -> Report {
    let page = reportcraft_model::PageSetup {
        paper: None,
        orientation: reportcraft_model::Orientation::Portrait,
        width_mm,
        height_mm: SUBREPORT_PAGE_MM,
        margin_mm: 0.0,
        columns: 1,
        column_gap_mm: 0.0,
        column_order: reportcraft_model::ColumnOrder::Down,
    };
    Report { page, page_header: Band::default(), page_footer: Band::default(), ..child.clone() }
}

/// The text a value matches on in a subreport link: `1`, `1.0` and `"1"` match.
fn link_key(v: &Value) -> String {
    v.to_string()
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

/// The height a group's bands take from `start` to `end` (header, nested groups or detail rows,
/// footer), assuming no page break.
fn group_height(report: &Report, rows: &RowSet, order: &[usize], start: usize, end: usize, level: usize, group_cols: &[usize]) -> f64 {
    let Some(group) = report.groups.get(level) else { return 0.0 };
    let mut h = group.header.height_mm + group.footer.height_mm;
    if level + 1 < group_cols.len() {
        let mut p = start;
        while p < end {
            let e = group_end(rows, order, p, level + 1, group_cols).min(end).max(p + 1);
            h += group_height(report, rows, order, p, e, level + 1, group_cols);
            p = e;
        }
    } else {
        h += (end - start) as f64 * report.detail.height_mm;
    }
    h
}

/// A group whose header has been printed and whose footer hasn't, by positions in the sorted rows.
#[derive(Clone, Copy)]
struct OpenGroup {
    level: usize,
    start: usize,
    end: usize,
}

/// The rows a band summarises, and the row its fields read.
#[derive(Clone, Copy)]
struct Scope<'a> {
    rows: &'a [usize],
    current: Option<usize>,
    /// The rows formulas summarise when they aren't `rows` (a detail band's group).
    group: Option<&'a [usize]>,
}

impl<'a> Scope<'a> {
    fn new(rows: &'a [usize], current: Option<usize>) -> Self {
        Self { rows, current, group: None }
    }
}

struct Writer<'a> {
    report: &'a Report,
    rows: &'a RowSet,
    params: &'a Params,
    locale: reportcraft_model::LocaleStyle,
    pages: Vec<Page>,
    page: Page,
    y: f64,
    /// Nothing but page and repeated headers on the page yet: no break would help.
    fresh: bool,
    /// Groups open at this point, outermost first.
    open: Vec<OpenGroup>,
    /// The column group and detail bands are going into, its top on this page, and the lowest
    /// point any column reached.
    column: usize,
    column_top: f64,
    column_bottom: f64,
    /// Height of the row of labels being filled (across order).
    row_height: f64,
    /// Items that show the page number, by page.
    page_number_items: Vec<(usize, usize)>,
    datasets: &'a Datasets,
    cancel: &'a Cancel,
    depth: usize,
    /// Where each band or subreport block starts (what a parent cuts a subreport at).
    marks: Vec<Mark>,
    /// Per subreport element: its dataset's rows by link key, and the report it lays out.
    link_index: HashMap<usize, HashMap<Vec<String>, Vec<usize>>>,
    child_reports: HashMap<usize, Arc<Report>>,
    /// Formula elements, parsed once (boxed: their address keys the aggregate memo).
    formulas: HashMap<usize, Option<Box<reportcraft_expr::Expr>>>,
    /// `visible_if` and `page_break_before_if` formulas, parsed once (by the address of their text).
    conditions: HashMap<usize, Option<Box<reportcraft_expr::Expr>>>,
    memo: formula::Memo,
    now: Option<chrono::NaiveDateTime>,
}

impl<'a> Writer<'a> {
    fn new(report: &'a Report, rows: &'a RowSet, ctx: &Context<'a>) -> Self {
        let locale = reportcraft_model::locale_or_default(&report.locale);
        Self {
            report,
            rows,
            params: ctx.params,
            datasets: ctx.datasets,
            cancel: ctx.cancel,
            depth: ctx.depth,
            marks: Vec::new(),
            link_index: HashMap::new(),
            child_reports: HashMap::new(),
            formulas: HashMap::new(),
            conditions: HashMap::new(),
            memo: formula::Memo::default(),
            now: formula::now(ctx.params),
            locale,
            pages: Vec::new(),
            page: Page::default(),
            y: 0.0,
            fresh: true,
            open: Vec::new(),
            column: 0,
            column_top: 0.0,
            column_bottom: 0.0,
            row_height: 0.0,
            page_number_items: Vec::new(),
        }
    }

    fn body_bottom(&self) -> f64 {
        self.report.page.size().1 - self.report.page.margin_mm - self.report.page_footer.height_mm
    }

    fn start_page(&mut self, with_header: bool, order: &[usize]) -> Result<()> {
        if self.pages.len() >= limits::MAX_PAGES {
            return Err(Error::TooLarge(format!("more than {} pages", limits::MAX_PAGES)));
        }
        self.page = Page::default();
        self.y = self.report.page.margin_mm;
        self.column = 0;
        self.row_height = 0.0;
        self.column_top = self.y;
        self.column_bottom = self.y;
        if with_header {
            self.page_header();
            self.repeat_headers(order);
        }
        self.fresh = true;
        Ok(())
    }

    /// Groups that continue on a new page or column say so again.
    fn repeat_headers(&mut self, order: &[usize]) {
        let open = self.open.clone();
        for g in open {
            let (Some(group), Some(scope)) = (self.report.groups.get(g.level), order.get(g.start..g.end)) else { continue };
            let scope = Scope::new(scope, order.get(g.start).copied());
            if group.repeat_header && !group.header.is_empty() && self.band_visible(&group.header, scope) {
                self.draw(&group.header, scope, true);
                self.y += group.header.height_mm;
                self.column_bottom = self.column_bottom.max(self.y);
            }
        }
    }

    fn new_page(&mut self, order: &[usize]) -> Result<()> {
        self.finish_page()?;
        self.start_page(true, order)
    }

    /// Where the body starts on a new page: below the page header and the headers repeated there.
    fn top_after_headers(&self) -> f64 {
        let repeated: f64 =
            self.open.iter().filter_map(|g| self.report.groups.get(g.level)).filter(|g| g.repeat_header).map(|g| g.header.height_mm).sum();
        self.report.page.margin_mm + self.report.page_header.height_mm + repeated
    }

    fn page_header(&mut self) {
        let report = self.report;
        // A hidden page header takes no room; it has no current row, only parameters to go by.
        if self.band_visible(&report.page_header, Scope::new(&[], None)) {
            self.draw(&report.page_header, Scope::new(&[], None), false);
            self.y += report.page_header.height_mm;
        }
        // Columns start below the page header (and below the report header on page one).
        self.column_top = self.y;
        self.column_bottom = self.y;
    }

    fn finish_page(&mut self) -> Result<()> {
        self.y = self.body_bottom();
        let report = self.report;
        // A hidden page footer keeps its room: the body ends at the same place on every page.
        if self.band_visible(&report.page_footer, Scope::new(&[], None)) {
            self.draw(&report.page_footer, Scope::new(&[], None), false);
        }
        self.pages.push(std::mem::take(&mut self.page));
        Ok(())
    }

    /// Places a band, moving to a new page first when it doesn't fit, unless the page is fresh
    /// (a break wouldn't give it more room, so it is placed rather than looping).
    fn band_in(&mut self, band: &Band, scope: Scope<'_>, order: &[usize]) -> Result<()> {
        self.band_in_keeping(band, scope, order, 0.0)
    }

    /// `band_in`, moving on first unless `following` millimetres also fit below the band (a band
    /// kept with the next one). A fresh page never moves on, so this can't loop.
    fn band_in_keeping(&mut self, band: &Band, scope: Scope<'_>, order: &[usize], following: f64) -> Result<()> {
        if band.is_empty() || !self.band_visible(band, scope) {
            return Ok(());
        }
        if !self.fresh && self.band_breaks(band, scope) {
            self.new_page(order)?;
        }
        if self.across() {
            // A group band starts a new row of labels and spans the first column.
            self.close_row();
            if !self.fresh && self.y + band.height_mm + following > self.body_bottom() + 1e-9 {
                self.new_page(order)?;
            }
            self.draw(band, scope, true);
            self.y += band.height_mm;
            self.column_bottom = self.column_bottom.max(self.y);
            self.fresh = false;
            return Ok(());
        }
        if !self.fresh && self.y + band.height_mm + following > self.body_bottom() + 1e-9 {
            self.next_column_or_page(true, order)?;
        }
        self.place(band, scope, true, order)?;
        self.column_bottom = self.column_bottom.max(self.y);
        self.fresh = false;
        Ok(())
    }

    /// Moves on to the next column, or the next page after the last column.
    fn next_column_or_page(&mut self, columned: bool, order: &[usize]) -> Result<()> {
        if columned && self.column + 1 < usize::from(self.report.page.columns) {
            self.column += 1;
            self.y = self.column_top;
            self.repeat_headers(order);
            self.fresh = true;
            Ok(())
        } else {
            self.new_page(order)
        }
    }

    /// The left edge of the current column, or of the body.
    fn left(&self, columned: bool) -> f64 {
        let page = &self.report.page;
        let column = if columned { self.column as f64 } else { 0.0 };
        page.margin_mm + column * (page.column_width() + page.column_gap_mm)
    }

    /// Draws a band at the current height and moves below it, flowing its subreport if it has one.
    fn place(&mut self, band: &Band, scope: Scope<'_>, columned: bool, order: &[usize]) -> Result<()> {
        let (top, item) = (self.y, self.page.items.len());
        self.draw(band, scope, columned);
        let mut subreport = None;
        for el in band.elements.iter().filter(|e| matches!(e.content, Content::Subreport { .. })) {
            if self.element_visible(el, scope) {
                subreport = Some(el);
            }
        }
        match subreport {
            None => {
                self.marks.push(Mark { item, top, height: band.height_mm });
                self.y += band.height_mm;
            }
            Some(el) => {
                self.marks.push(Mark { item, top, height: el.y_mm });
                self.flow_subreport(el, band, top, scope, columned, order)?;
            }
        }
        Ok(())
    }

    /// Lays a subreport out below its band's top, block by block, moving to the next column or
    /// page between blocks as needed. The band ends below the subreport, keeping the room the
    /// band had under the element, and never above the band's own bottom.
    fn flow_subreport(&mut self, el: &Element, band: &Band, top: f64, scope: Scope<'_>, columned: bool, order: &[usize]) -> Result<()> {
        let Content::Subreport { report: child, dataset, links, hide_when_empty } = &el.content else { return Ok(()) };
        if self.depth + 1 >= reportcraft_model::MAX_SUBREPORT_DEPTH {
            return Err(Error::TooLarge(format!("reports nest at most {} deep", reportcraft_model::MAX_SUBREPORT_DEPTH)));
        }
        let rows = self.subreport_rows(el, dataset, links, scope)?;
        let blocks = if rows.rows.is_empty() && *hide_when_empty {
            Vec::new()
        } else {
            let key = std::ptr::from_ref(el) as usize;
            let report = Arc::clone(self.child_reports.entry(key).or_insert_with(|| Arc::new(subreport_report(child, el.width_mm))));
            let ctx = Context { datasets: self.datasets, params: self.params, cancel: self.cancel, depth: self.depth + 1 };
            subreport_blocks(&report, &rows, &ctx)?
        };
        let below = (band.height_mm - el.y_mm - el.height_mm).max(0.0);
        let mut y = top + el.y_mm;
        let (mut moved, mut fresh) = (false, false);
        for block in blocks {
            if !fresh && y + block.height > self.body_bottom() + 1e-9 {
                self.next_column_or_page(columned, order)?;
                y = self.y;
                moved = true;
                fresh = true;
            } else {
                fresh = false;
            }
            let left = self.left(columned) + el.x_mm;
            self.marks.push(Mark { item: self.page.items.len(), top: y, height: block.height });
            for mut it in block.items {
                it.x_mm += left;
                it.y_mm += y;
                self.page.items.push(it);
            }
            y += block.height;
        }
        let end = if moved { y + below } else { (top + band.height_mm).max(y + below) };
        self.marks.push(Mark { item: self.page.items.len(), top: y, height: end - y });
        self.y = end;
        Ok(())
    }

    /// The subreport's rows: its whole dataset, or with links the rows matching the current row.
    fn subreport_rows(&mut self, el: &Element, dataset: &str, links: &[SubreportLink], scope: Scope<'_>) -> Result<RowSet> {
        let data = Arc::clone(self.datasets.get(dataset).ok_or_else(|| Error::NotFound(format!("subreport dataset `{dataset}` (load it first)")))?);
        if links.is_empty() {
            return Ok((*data).clone());
        }
        let parent_cols = links
            .iter()
            .map(|l| self.rows.column_index(&l.parent).ok_or_else(|| Error::NotFound(format!("subreport link field `{}`", l.parent))))
            .collect::<Result<Vec<_>>>()?;
        let child_cols = links
            .iter()
            .map(|l| data.column_index(&l.child).ok_or_else(|| Error::NotFound(format!("subreport link field `{}` in `{dataset}`", l.child))))
            .collect::<Result<Vec<_>>>()?;
        let empty = RowSet { columns: data.columns.clone(), rows: Vec::new() };
        let Some(current) = scope.current else { return Ok(empty) };
        let parent = row_of(self.rows, current);
        // A missing value links to nothing.
        if parent_cols.iter().any(|&c| self.rows.value(parent, c).is_null()) {
            return Ok(empty);
        }
        let key: Vec<String> = parent_cols.iter().map(|&c| link_key(self.rows.value(parent, c))).collect();
        let index = self.link_index.entry(std::ptr::from_ref(el) as usize).or_insert_with(|| {
            let mut index: HashMap<Vec<String>, Vec<usize>> = HashMap::new();
            for (i, r) in data.rows.iter().enumerate() {
                if child_cols.iter().any(|&c| data.value(r, c).is_null()) {
                    continue;
                }
                index.entry(child_cols.iter().map(|&c| link_key(data.value(r, c))).collect()).or_default().push(i);
            }
            index
        });
        let rows = index.get(&key).map(|ix| ix.iter().filter_map(|&i| data.rows.get(i).cloned()).collect()).unwrap_or_default();
        Ok(RowSet { rows, ..empty })
    }

    fn across(&self) -> bool {
        self.report.page.columns > 1 && self.report.page.column_order == reportcraft_model::ColumnOrder::Across
    }

    /// Moves below a partly filled row of labels.
    fn close_row(&mut self) {
        if self.column > 0 {
            self.y += self.row_height;
            self.column = 0;
        }
        self.row_height = 0.0;
    }

    /// Places a detail band: down a column, or in the next cell of a row of labels.
    fn detail(&mut self, band: &Band, scope: Scope<'_>, order: &[usize]) -> Result<()> {
        if !self.across() {
            return self.band_in(band, scope, order);
        }
        if band.is_empty() || !self.band_visible(band, scope) {
            return Ok(());
        }
        if !self.fresh && self.band_breaks(band, scope) {
            self.new_page(order)?;
        }
        if self.column == 0 && !self.fresh && self.y + band.height_mm > self.body_bottom() + 1e-9 {
            self.new_page(order)?;
        }
        self.draw(band, scope, true);
        self.row_height = self.row_height.max(band.height_mm);
        self.column_bottom = self.column_bottom.max(self.y + self.row_height);
        self.fresh = false;
        self.column += 1;
        if self.column >= usize::from(self.report.page.columns) {
            self.close_row();
        }
        Ok(())
    }

    /// Places a band that spans the page (report header or footer) below every column.
    fn full_band(&mut self, band: &Band, scope: Scope<'_>, order: &[usize]) -> Result<()> {
        if band.is_empty() || !self.band_visible(band, scope) {
            return Ok(());
        }
        self.close_row();
        self.y = self.y.max(self.column_bottom);
        self.column = 0;
        if !self.fresh && (self.band_breaks(band, scope) || self.y + band.height_mm > self.body_bottom() + 1e-9) {
            self.new_page(order)?;
        }
        self.place(band, scope, false, order)?;
        self.column_top = self.y;
        self.column_bottom = self.y;
        self.fresh = false;
        Ok(())
    }

    /// Closes the open groups from the innermost down to `level`, at the row before `pos`.
    fn close_groups(&mut self, order: &[usize], pos: usize, level: usize) -> Result<()> {
        let last = pos.checked_sub(1).and_then(|p| order.get(p)).copied();
        while let Some(g) = self.open.last().copied().filter(|g| g.level >= level) {
            let footer = self.report.groups.get(g.level).map(|gr| &gr.footer);
            if let (Some(footer), Some(scope)) = (footer, order.get(g.start..pos)) {
                self.band_in(footer, Scope::new(scope, last), order)?;
            }
            self.open.pop();
        }
        Ok(())
    }

    /// Draws a band at the current height; `columned` bands go into the current column.
    fn draw(&mut self, band: &Band, scope: Scope<'_>, columned: bool) {
        let page = &self.report.page;
        let column = if columned { self.column as f64 } else { 0.0 };
        let left = page.margin_mm + column * (page.column_width() + page.column_gap_mm);
        for el in &band.elements {
            if !self.element_visible(el, scope) {
                continue;
            }
            if matches!(el.content, Content::PageNumber) {
                self.page_number_items.push((self.pages.len(), self.page.items.len()));
            }
            if let Content::Subreport { .. } = &el.content {
                // Laid out by `place`, below the band's top.
                continue;
            }
            if let Content::Barcode { .. } = &el.content {
                self.barcode(el, scope, left);
                continue;
            }
            if let Content::Formula { expression } = &el.content {
                self.formulas
                    .entry(std::ptr::from_ref(&el.content) as usize)
                    .or_insert_with(|| reportcraft_expr::parse(expression).ok().map(Box::new));
            }
            let (text, cell) = self.eval(&el.content, el.format.as_ref(), scope);
            let style = self.style_of(el, scope);
            let shape = match &el.content {
                Content::Line { thickness_mm, color } => Some(Shape::Line { thickness_mm: *thickness_mm, color: color.clone() }),
                Content::Rect { border_mm, border_color, fill } => {
                    Some(Shape::Rect { border_mm: *border_mm, border_color: border_color.clone(), fill: fill.clone() })
                }
                _ => None,
            };
            self.page.items.push(Item {
                shape,
                x_mm: left + el.x_mm,
                y_mm: self.y + el.y_mm,
                width_mm: el.width_mm,
                height_mm: el.height_mm,
                style,
                text,
                cell,
            });
        }
    }

    /// A barcode element: the symbol, the text under a linear one, or the value as text when the
    /// symbology can't hold it.
    fn barcode(&mut self, el: &reportcraft_model::Element, scope: Scope<'_>, left: f64) {
        let Content::Barcode { symbology, field, text, show_text, ec_level } = &el.content else { return };
        let value = match (field, text) {
            (Some(f), _) => match (self.rows.column_index(f), scope.current) {
                (Some(c), Some(r)) => self.rows.value(row_of(self.rows, r), c).to_string(),
                _ => String::new(),
            },
            (None, Some(t)) => t.clone(),
            (None, None) => String::new(),
        };
        let (x, y) = (left + el.x_mm, self.y + el.y_mm);
        let item = |y_mm: f64, height_mm: f64, text: String, shape: Option<Shape>| Item {
            x_mm: x,
            y_mm,
            width_mm: el.width_mm,
            height_mm,
            style: el.style,
            text,
            shape,
            cell: None,
        };
        if value.trim().is_empty() {
            return;
        }
        let symbol = match reportcraft_barcode::encode(*symbology, &value, *ec_level) {
            Ok(s) => s,
            Err(_) => {
                self.page.items.push(item(y, el.height_mm, value, None));
                return;
            }
        };
        let with_text = symbology.is_linear() && *show_text && el.height_mm >= 2.0 * BARCODE_TEXT_MM;
        let bars_height = if with_text { el.height_mm - BARCODE_TEXT_MM } else { el.height_mm };
        let modules = symbol.modules.iter().map(|&d| if d { '1' } else { '0' }).collect();
        let shape = Shape::Barcode { columns: symbol.columns, rows: symbol.rows, quiet: symbol.quiet, modules };
        self.page.items.push(item(y, bars_height, String::new(), Some(shape)));
        if with_text {
            let mut under = item(y + bars_height, BARCODE_TEXT_MM, symbol.text, None);
            under.style.size_pt = 8.0;
            under.style.align = reportcraft_model::Align::Center;
            self.page.items.push(under);
        }
    }

    /// The text an element shows, and the typed value behind it when there is one.
    fn eval(&self, content: &Content, format: Option<&reportcraft_model::Format>, scope: Scope<'_>) -> (String, Option<Cell>) {
        let column = |field: &str| self.rows.column_index(field);
        let none = || (String::new(), None);
        match content {
            Content::Text { text } => (text.clone(), None),
            Content::Field { field } => match (column(field), scope.current) {
                (Some(c), Some(r)) => self.typed(self.rows.value(row_of(self.rows, r), c).clone(), format),
                _ => none(),
            },
            Content::Aggregate { function, field } => match column(field) {
                Some(c) => {
                    let total = aggregate(*function, scope.rows.iter().map(|&r| self.rows.value(row_of(self.rows, r), c)));
                    // A count is a count: a currency or date format on the field doesn't apply to it.
                    let format = if *function == AggregateFn::Count { None } else { format };
                    self.typed(total, format)
                }
                None => none(),
            },
            Content::Parameter { name } => {
                let raw = self
                    .params
                    .get(name)
                    .cloned()
                    .or_else(|| self.report.parameters.iter().find(|p| &p.name == name).and_then(|p| p.default.clone()));
                raw.map(|r| self.typed(Value::Text(r), format)).unwrap_or_else(none)
            }
            Content::Formula { .. } => {
                // Parsed by `draw`; a report that passed validation always parses.
                let Some(Some(expr)) = self.formulas.get(&(std::ptr::from_ref(content) as usize)) else { return none() };
                let env = formula::BandEnv {
                    report: self.report,
                    rows: self.rows,
                    current: scope.current,
                    summary: scope.group.unwrap_or(scope.rows),
                    params: self.params,
                    now: self.now,
                    memo: &self.memo,
                    locale: reportcraft_expr::Locale { decimal: self.locale.decimal, group: self.locale.group, names: self.locale.names },
                };
                self.typed(formula::to_value(reportcraft_expr::evaluate(expr, &env)), format)
            }
            Content::RefreshedAt | Content::PrintedAt => {
                let key = if matches!(content, Content::RefreshedAt) { REFRESHED_AT } else { PRINTED_AT };
                let default = reportcraft_model::Format::Date { pattern: Some(format!("{} %H:%M", self.locale.date_pattern)) };
                self.params.get(key).map(|t| self.typed(Value::Text(t.clone()), Some(format.unwrap_or(&default)))).unwrap_or_else(none)
            }
            Content::PageNumber | Content::Line { .. } | Content::Rect { .. } | Content::Barcode { .. } | Content::Subreport { .. } => none(),
        }
    }

    /// Whether a condition formula holds for `scope`: `Some(true)` for TRUE or a non-zero
    /// number, `Some(false)` for FALSE, 0 or a blank, `None` for anything else (an error, text).
    fn holds(&mut self, condition: &String, scope: Scope<'_>) -> Option<bool> {
        match self.value_of(condition, scope)? {
            reportcraft_expr::Val::Bool(b) => Some(b),
            reportcraft_expr::Val::Number(n) => Some(n != 0.0),
            reportcraft_expr::Val::Blank => Some(false),
            _ => None,
        }
    }

    /// The style an element is drawn with: its own, with the colours its formulas give for
    /// `scope` (a result that isn't a colour keeps the style's).
    fn style_of(&mut self, el: &Element, scope: Scope<'_>) -> TextStyle {
        let mut style = el.style;
        let mut colour = |formula: &Option<String>| match formula {
            Some(f) => match self.value_of(f, scope) {
                Some(reportcraft_expr::Val::Text(t)) => reportcraft_model::Color::parse(t.trim()),
                _ => None,
            },
            None => None,
        };
        if let Some(c) = colour(&el.color_formula) {
            style.color = Some(c);
        }
        if let Some(c) = colour(&el.background_formula) {
            style.background = Some(c);
        }
        style
    }

    /// A band formula's value for `scope` (`visible_if`, a colour formula…), parsed once.
    fn value_of(&mut self, formula: &String, scope: Scope<'_>) -> Option<reportcraft_expr::Val> {
        let key = std::ptr::from_ref(formula) as usize;
        self.conditions.entry(key).or_insert_with(|| reportcraft_expr::parse(formula).ok().map(Box::new));
        let expr = self.conditions.get(&key)?.as_deref()?;
        let env = formula::BandEnv {
            report: self.report,
            rows: self.rows,
            current: scope.current,
            summary: scope.group.unwrap_or(scope.rows),
            params: self.params,
            now: self.now,
            memo: &self.memo,
            locale: reportcraft_expr::Locale { decimal: self.locale.decimal, group: self.locale.group, names: self.locale.names },
        };
        Some(reportcraft_expr::evaluate(expr, &env))
    }

    /// A band is printed unless its `visible_if` says otherwise (an error shows it).
    fn band_visible(&mut self, band: &Band, scope: Scope<'_>) -> bool {
        band.visible_if.as_ref().is_none_or(|c| self.holds(c, scope).unwrap_or(true))
    }

    /// A band starts a new page when its `page_break_before_if` holds (an error doesn't break).
    fn band_breaks(&mut self, band: &Band, scope: Scope<'_>) -> bool {
        band.page_break_before_if.as_ref().is_some_and(|c| self.holds(c, scope).unwrap_or(false))
    }

    fn element_visible(&mut self, el: &Element, scope: Scope<'_>) -> bool {
        el.visible_if.as_ref().is_none_or(|c| self.holds(c, scope).unwrap_or(true))
    }

    /// A value's text with `format`, keeping numbers, booleans and dates (text under a date
    /// format) typed.
    fn typed(&self, value: Value, format: Option<&reportcraft_model::Format>) -> (String, Option<Cell>) {
        let text = format::write(&value, format, &self.locale);
        let keep = match &value {
            Value::Int(_) | Value::Number(_) | Value::Bool(_) => true,
            Value::Text(_) => matches!(format, Some(reportcraft_model::Format::Date { .. })),
            Value::Null => false,
        };
        (text, keep.then(|| Cell { value, format: format.cloned() }))
    }

    fn into_document(mut self) -> Document {
        let total = self.pages.len();
        for (page, item) in std::mem::take(&mut self.page_number_items) {
            if let Some(it) = self.pages.get_mut(page).and_then(|p| p.items.get_mut(item)) {
                it.text = format!("{} / {total}", page + 1);
            }
        }
        let (width_mm, height_mm) = self.report.page.size();
        Document { title: self.report.title.clone(), width_mm, height_mm, margin_mm: self.report.page.margin_mm, pages: self.pages }
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
        Element {
            x_mm: 0.0,
            y_mm: 0.0,
            width_mm: 50.0,
            height_mm: 5.0,
            style: TextStyle::default(),
            content,
            format: None,
            visible_if: None,
            background_formula: None,
            color_formula: None,
        }
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
            keep_together: false,
            repeat_header: false,
            page_break_before: false,
            header: Band {
                height_mm: 6.0,
                elements: vec![el(Content::Field { field: "branch".into() })],
                page_break_before_if: None,
                visible_if: None,
                keep_with_next: false,
            },
            footer: Band {
                height_mm: 6.0,
                elements: vec![el(Content::Aggregate { function: AggregateFn::Sum, field: "amount".into() })],
                page_break_before_if: None,
                visible_if: None,
                keep_with_next: false,
            },
        });
        r.detail = Band {
            height_mm: 5.0,
            elements: vec![el(Content::Field { field: "amount".into() })],
            page_break_before_if: None,
            visible_if: None,
            keep_with_next: false,
        };
        r.report_footer = Band {
            height_mm: 6.0,
            elements: vec![el(Content::Aggregate { function: AggregateFn::Sum, field: "amount".into() })],
            page_break_before_if: None,
            visible_if: None,
            keep_with_next: false,
        };
        let doc = layout(&r, &rows(), &Params::new()).unwrap();
        assert_eq!(texts(&doc), ["Lyon", "10", "2.5", "12.5", "Paris", "5", "5", "17.5"]);
    }

    #[test]
    fn breaks_pages_and_numbers_them() {
        let mut r = Report::new("t");
        r.page.height_mm = 60.0;
        r.page.margin_mm = 10.0;
        r.page_footer =
            Band { height_mm: 5.0, elements: vec![el(Content::PageNumber)], page_break_before_if: None, visible_if: None, keep_with_next: false };
        r.detail = Band {
            height_mm: 15.0,
            elements: vec![el(Content::Field { field: "branch".into() })],
            page_break_before_if: None,
            visible_if: None,
            keep_with_next: false,
        };
        let doc = layout(&r, &rows(), &Params::new()).unwrap();
        assert_eq!(doc.pages.len(), 2);
        assert!(texts(&doc).contains(&"2 / 2".to_owned()));
    }

    #[test]
    fn group_options_never_make_blank_pages_or_loop() {
        let many = RowSet {
            columns: rows().columns,
            rows: (0..40).map(|i| vec![Value::Text(if i < 30 { "Lyon" } else { "Paris" }.into()), Value::Number(1.0)]).collect(),
        };
        let mut r = Report::new("t");
        r.page.height_mm = 60.0;
        r.page.margin_mm = 5.0;
        r.groups.push(Group {
            field: "branch".into(),
            keep_together: true,
            repeat_header: true,
            page_break_before: true,
            header: Band {
                height_mm: 6.0,
                elements: vec![el(Content::Field { field: "branch".into() })],
                page_break_before_if: None,
                visible_if: None,
                keep_with_next: false,
            },
            footer: Band::default(),
        });
        r.detail = Band {
            height_mm: 5.0,
            elements: vec![el(Content::Field { field: "amount".into() })],
            page_break_before_if: None,
            visible_if: None,
            keep_with_next: false,
        };
        let doc = layout(&r, &many, &Params::new()).unwrap();
        // Body 50 mm: Lyon (6 + 30 x 5 = 156 mm) can't be kept together, so it flows from page one,
        // repeating its header (8 rows per page after it); Paris starts a new page.
        assert!(doc.pages.iter().all(|p| !p.items.is_empty()), "no blank page");
        let first_texts: Vec<&str> = doc.pages.iter().filter_map(|p| p.items.first().map(|i| i.text.as_str())).collect();
        assert_eq!(first_texts, ["Lyon", "Lyon", "Lyon", "Lyon", "Paris", "Paris"]);
    }

    #[test]
    fn refresh_and_print_times_follow_the_locale() {
        let mut report = Report::new("r");
        report.locale = "fr".into();
        report.report_header = Band {
            height_mm: 6.0,
            elements: vec![el(Content::RefreshedAt), el(Content::PrintedAt)],
            page_break_before_if: None,
            visible_if: None,
            keep_with_next: false,
        };
        let mut params = Params::new();
        params.insert(REFRESHED_AT.into(), "2026-10-01T14:05:09+02:00".into());
        let doc = layout(&report, &rows(), &params).unwrap();
        let t = texts(&doc);
        assert!(t.contains(&"01/10/2026 14:05".to_owned()), "{t:?}");
        assert!(!t.iter().any(|x| x.contains("2026-")), "no printing time given: nothing shown");
    }

    #[test]
    fn a_cancelled_render_stops() {
        let report = Report::new("r");
        let cancel = Cancel::default();
        assert!(layout_with_cancel(&report, &rows(), &Params::new(), &cancel).is_ok());
        cancel.cancel();
        assert!(matches!(layout_with_cancel(&report, &rows(), &Params::new(), &cancel), Err(Error::Cancelled(_))));
        let late = Cancel::default().with_time_limit(std::time::Duration::ZERO);
        assert!(matches!(layout_with_cancel(&report, &rows(), &Params::new(), &late), Err(Error::TooLarge(_))));
    }

    #[test]
    fn named_formulas_are_fields_and_formula_elements_summarise_their_band() {
        let f = |e: &str| el(Content::Formula { expression: e.into() });
        let mut r = Report::new("t");
        r.formulas = vec![
            reportcraft_model::FormulaField { name: "net".into(), expression: "[gross] * 0.8".into() },
            reportcraft_model::FormulaField { name: "gross".into(), expression: "[amount] * 10".into() },
            reportcraft_model::FormulaField { name: "size".into(), expression: "IF([gross] >= 50, \"big\", \"small\")".into() },
        ];
        r.groups.push(Group {
            field: "size".into(),
            keep_together: false,
            repeat_header: false,
            page_break_before: false,
            header: Band {
                height_mm: 6.0,
                elements: vec![f("UPPER([size]) & \" (\" & COUNT([net]) & \")\"")],
                page_break_before_if: None,
                visible_if: None,
                keep_with_next: false,
            },
            footer: Band { height_mm: 6.0, elements: vec![f("SUM([net])")], page_break_before_if: None, visible_if: None, keep_with_next: false },
        });
        r.sort.push(reportcraft_model::SortKey { field: "net".into(), descending: true });
        let share = Element { format: Some(reportcraft_model::Format::Percent { decimals: 0 }), ..f("[net] / SUM([net])") };
        r.detail = Band {
            height_mm: 5.0,
            elements: vec![el(Content::Field { field: "net".into() }), share],
            page_break_before_if: None,
            visible_if: None,
            keep_with_next: false,
        };
        r.report_footer = Band {
            height_mm: 6.0,
            elements: vec![f("\"Total: \" & SUM([net]) & \" over \" & COUNTIF([size], \"big\") & \" big\"")],
            page_break_before_if: None,
            visible_if: None,
            keep_with_next: false,
        };
        let doc = layout(&r, &rows(), &Params::new()).unwrap();
        assert_eq!(texts(&doc), ["BIG (2)", "80", "67%", "40", "33%", "120", "SMALL (1)", "20", "100%", "20", "Total: 140 over 2 big"]);
    }

    #[test]
    fn formulas_read_typed_parameters_and_the_render_clock() {
        let mut r = Report::new("t");
        r.parameters = vec![
            reportcraft_model::Parameter {
                kind: reportcraft_model::ParamType::Number,
                ..reportcraft_model::Parameter::text("min", "", Some("4".into()))
            },
            reportcraft_model::Parameter { kind: reportcraft_model::ParamType::DateRange, ..reportcraft_model::Parameter::text("period", "", None) },
        ];
        let f = |e: &str| el(Content::Formula { expression: e.into() });
        r.report_header = Band {
            height_mm: 6.0,
            elements: vec![
                f("COUNTIF([amount], \">\" & PARAM(\"min\"))"),
                f("PARAM(\"period.to\") - PARAM(\"period.from\")"),
                f("TODAY() - PARAM(\"period.from\")"),
                Element {
                    format: Some(reportcraft_model::Format::Date { pattern: Some("%d/%m/%Y".into()) }),
                    ..f("EOMONTH(PARAM(\"period.from\"), 0)")
                },
                f("1/0"),
            ],
            page_break_before_if: None,
            visible_if: None,
            keep_with_next: false,
        };
        let mut params = Params::new();
        params.insert("period".into(), "2026-10-01..2026-10-31".into());
        params.insert(PRINTED_AT.into(), "2026-11-11T09:30:00+01:00".into());
        assert_eq!(texts(&layout(&r, &rows(), &params).unwrap()), ["2", "30", "41", "31/10/2026", "#DIV/0!"]);
    }

    #[test]
    fn formulas_write_numbers_and_dates_in_the_report_language() {
        let mut r = Report::new("t");
        r.locale = "fr".into();
        let f = |e: &str| el(Content::Formula { expression: e.into() });
        r.report_footer = Band {
            height_mm: 6.0,
            elements: vec![
                f("\"Total : \" & SUM([amount]) * 100"),
                f("TEXT(SUM([amount]) * 100, \"#,##0.00 €\")"),
                f("TEXT(DATE(2026,11,11), \"dddd d mmmm\")"),
                f("VALUE(\"12,5\") * 2"),
            ],
            ..Band::default()
        };
        assert_eq!(texts(&layout(&r, &rows(), &Params::new()).unwrap()), ["Total : 1750", "1\u{202f}750,00 €", "mercredi 11 novembre", "25"]);
        r.locale = "en".into();
        assert_eq!(texts(&layout(&r, &rows(), &Params::new()).unwrap())[1], "1,750.00 €");
    }

    #[test]
    fn a_header_kept_with_the_next_band_never_ends_a_page() {
        let mut r = Report::new("t");
        // Body from 5 to 55 mm. After an 18 mm report header and Lyon (6 + 2 x 5 + 6 = 22 mm),
        // Paris' header ends at 51 mm and its first row would end at 56 mm: the header fits, its
        // row doesn't.
        r.page.height_mm = 60.0;
        r.page.margin_mm = 5.0;
        r.report_header = Band { height_mm: 18.0, elements: vec![el(Content::Text { text: "Title".into() })], ..Band::default() };
        r.groups.push(Group {
            field: "branch".into(),
            keep_together: false,
            repeat_header: false,
            page_break_before: false,
            header: Band { height_mm: 6.0, elements: vec![el(Content::Field { field: "branch".into() })], ..Band::default() },
            footer: Band { height_mm: 6.0, elements: vec![el(Content::Text { text: "end".into() })], ..Band::default() },
        });
        r.detail = Band { height_mm: 5.0, elements: vec![el(Content::Field { field: "amount".into() })], ..Band::default() };
        let pages = |r: &Report| -> Vec<Vec<String>> {
            layout(r, &rows(), &Params::new()).unwrap().pages.iter().map(|p| p.items.iter().map(|i| i.text.clone()).collect()).collect()
        };
        assert_eq!(pages(&r), [vec!["Title", "Lyon", "10", "2.5", "end", "Paris"], vec!["5", "end"]], "the header alone at the bottom");
        if let Some(g) = r.groups.first_mut() {
            g.header.keep_with_next = true;
        }
        assert_eq!(pages(&r), [vec!["Title", "Lyon", "10", "2.5", "end"], vec!["Paris", "5", "end"]], "kept with its first row");
    }

    #[test]
    fn colour_formulas_colour_each_row() {
        let mut r = Report::new("t");
        let red = reportcraft_model::Color(0xcc, 0, 0);
        let style = TextStyle { background: Some(reportcraft_model::Color(0xee, 0xee, 0xee)), ..TextStyle::default() };
        r.detail = Band {
            height_mm: 5.0,
            elements: vec![Element {
                style,
                color_formula: Some("IF([amount] < 5, \"#c00\", \"not a colour\")".into()),
                ..el(Content::Field { field: "amount".into() })
            }],
            ..Band::default()
        };
        let doc = layout(&r, &rows(), &Params::new()).unwrap();
        let colours: Vec<_> = doc.pages.iter().flat_map(|p| &p.items).map(|i| (i.text.clone(), i.style.color, i.style.background)).collect();
        let grey = style.background;
        assert_eq!(colours, [("10".into(), None, grey), ("5".into(), None, grey), ("2.5".into(), Some(red), grey)]);
    }

    #[test]
    fn a_formula_is_evaluated_before_it_is_placed() {
        let mut r = Report::new("t");
        r.formulas = vec![reportcraft_model::FormulaField { name: "double".into(), expression: "[amount] * 2".into() }];
        let v = evaluate_formula(&r, &rows(), &Params::new(), "[double] / SUM([double])", 2).unwrap();
        assert_eq!(v, [Value::Number(0.571428571428571), Value::Number(0.285714285714286)]);
        assert_eq!(evaluate_formula(&r, &RowSet::default(), &Params::new(), "SUM([x]) + 1", 5).unwrap(), [Value::Text("#NAME?".into())]);
        assert!(matches!(evaluate_formula(&r, &rows(), &Params::new(), "1 +", 5), Err(Error::Invalid(_))));
    }

    #[test]
    fn continuous_layout_is_one_page_without_breaks_or_footers() {
        let mut r = Report::new("t");
        r.page.height_mm = 60.0;
        r.page.columns = 2;
        r.page_header = Band {
            height_mm: 5.0,
            elements: vec![el(Content::Text { text: "Amount".into() })],
            page_break_before_if: None,
            visible_if: None,
            keep_with_next: false,
        };
        r.page_footer =
            Band { height_mm: 5.0, elements: vec![el(Content::PageNumber)], page_break_before_if: None, visible_if: None, keep_with_next: false };
        let number = reportcraft_model::Format::Number { decimals: 1, grouping: true };
        r.detail = Band {
            height_mm: 15.0,
            elements: vec![Element { format: Some(number.clone()), ..el(Content::Field { field: "amount".into() }) }],
            page_break_before_if: None,
            visible_if: None,
            keep_with_next: false,
        };
        let doc = render_continuous(&r, &rows(), &Datasets::new(), &Params::new(), &Cancel::default()).unwrap();
        assert_eq!(doc.pages.len(), 1);
        assert_eq!(texts(&doc), ["Amount", "10.0", "5.0", "2.5"]);
        let first = doc.pages.first().and_then(|p| p.items.get(1)).unwrap();
        assert_eq!((first.x_mm, first.y_mm), (0.0, 5.0), "no margin, one column");
        assert_eq!(first.cell, Some(Cell { value: Value::Number(10.0), format: Some(number) }));
        assert!((doc.height_mm - 40.0).abs() < 1e-9, "down to the last item");
        assert_eq!(layout(&r, &rows(), &Params::new()).unwrap().pages.len(), 2, "paginated, for comparison");
    }

    #[test]
    fn conditions_hide_bands_and_elements_and_break_pages() {
        let cond = |c: &str| Some(c.to_owned());
        let mut r = Report::new("t");
        r.parameters = vec![reportcraft_model::Parameter::text("title", "", None)];
        r.page_header = Band {
            height_mm: 5.0,
            elements: vec![el(Content::Text { text: "Header".into() })],
            visible_if: cond("PARAM(\"title\") <> \"\""),
            ..Band::default()
        };
        r.groups.push(Group {
            field: "branch".into(),
            keep_together: false,
            repeat_header: false,
            page_break_before: false,
            header: Band {
                height_mm: 6.0,
                elements: vec![el(Content::Field { field: "branch".into() })],
                page_break_before_if: cond("[branch] = \"Paris\""),
                ..Band::default()
            },
            footer: Band {
                height_mm: 6.0,
                elements: vec![el(Content::Aggregate { function: AggregateFn::Sum, field: "amount".into() })],
                visible_if: cond("COUNT([amount]) > 1"),
                ..Band::default()
            },
        });
        let big = Element { visible_if: cond("[amount] > 6"), ..el(Content::Text { text: "big".into() }) };
        r.detail = Band {
            height_mm: 5.0,
            elements: vec![el(Content::Field { field: "amount".into() }), big],
            visible_if: cond("[amount] >= 2.5"),
            ..Band::default()
        };
        // An error never hides data or breaks a page.
        r.report_footer = Band {
            height_mm: 6.0,
            elements: vec![el(Content::Text { text: "end".into() })],
            visible_if: cond("1/0"),
            page_break_before_if: cond("1/0"),
            keep_with_next: false,
        };
        let doc = layout(&r, &rows(), &Params::new()).unwrap();
        let pages: Vec<Vec<String>> = doc.pages.iter().map(|p| p.items.iter().map(|i| i.text.clone()).collect()).collect();
        assert_eq!(pages, [vec!["Lyon", "10", "big", "2.5", "12.5"], vec!["Paris", "5", "end"]]);
        let first = doc.pages.first().and_then(|p| p.items.first()).unwrap();
        assert!((first.y_mm - 15.0).abs() < 1e-9, "a hidden page header takes no room");
        let mut params = Params::new();
        params.insert("title".into(), "Q4".into());
        let doc = layout(&r, &rows(), &params).unwrap();
        assert_eq!(doc.pages.iter().filter(|p| p.items.iter().any(|i| i.text == "Header")).count(), 2, "shown when the parameter is given");
        // Hiding every detail row leaves the groups.
        r.detail.visible_if = cond("FALSE");
        assert_eq!(texts(&layout(&r, &rows(), &Params::new()).unwrap()), ["Lyon", "12.5", "Paris", "end"]);
    }

    #[test]
    fn page_ranges_pick_pages_in_order() {
        let doc = Document {
            title: "t".into(),
            width_mm: 10.0,
            height_mm: 10.0,
            margin_mm: 0.0,
            pages: (1..=6).map(|n| Page { items: vec![Item { text: n.to_string(), ..reportcraft_test_item() }] }).collect(),
        };
        let picked = |spec: &str| select_pages(&doc, spec).map(|d| texts(&d).join(","));
        assert_eq!(picked("1-3, 5").unwrap(), "1,2,3,5");
        assert_eq!(picked("5,1,5").unwrap(), "1,5", "document order, each page once");
        assert_eq!(picked("5-").unwrap(), "5,6");
        assert_eq!(picked("-2").unwrap(), "1,2");
        assert_eq!(picked("4-99").unwrap(), "4,5,6", "past the end is ignored");
        assert!(picked("8-").unwrap_err().to_string().contains("none of the 6"));
        assert!(picked("9").unwrap_err().to_string().contains("none of the 6"));
        assert!(picked("3-1").unwrap_err().to_string().contains("ends before"));
        for bad in ["0", "a", "1-2-3", "", " , "] {
            assert!(picked(bad).is_err(), "{bad}");
        }
    }

    fn reportcraft_test_item() -> Item {
        Item { x_mm: 0.0, y_mm: 0.0, width_mm: 1.0, height_mm: 1.0, style: TextStyle::default(), text: String::new(), shape: None, cell: None }
    }

    #[test]
    fn unknown_group_field_is_an_error_and_empty_data_still_renders() {
        let mut r = Report::new("t");
        r.groups.push(Group {
            field: "nope".into(),
            keep_together: false,
            repeat_header: false,
            page_break_before: false,
            header: Band::default(),
            footer: Band::default(),
        });
        assert!(matches!(layout(&r, &rows(), &Params::new()), Err(Error::NotFound(_))));
        let doc = layout(&Report::new("t"), &RowSet::default(), &Params::new()).unwrap();
        assert_eq!(doc.pages.len(), 1);
    }
}
