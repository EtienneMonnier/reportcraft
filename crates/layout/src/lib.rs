//! ReportCraft layout (L3): lays a report and its rows out as pages of positioned text,
//! toolkit-free. Writers (HTML here; PDF next, through craft-libs' craft-pdf) read `Document`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod format;
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
    Ok(run(report, rows, &ctx)?.into_document())
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
    w.full_band(&report.report_header, Scope { rows: &order, current: order.first().copied() }, &order)?;
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
                w.band_in(&group.header, Scope { rows: scope, current: Some(row) }, &order)?;
            }
        }
        w.detail(&report.detail, Scope { rows: std::slice::from_ref(&row), current: Some(row) }, &order)?;
    }
    if !order.is_empty() {
        w.close_groups(&order, order.len(), 0)?;
    }
    w.full_band(&report.report_footer, Scope { rows: &order, current: order.last().copied() }, &order)?;
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
    let w = run(child, rows, ctx)?;
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
            if group.repeat_header && !group.header.is_empty() {
                self.draw(&group.header, Scope { rows: scope, current: order.get(g.start).copied() }, true);
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
        self.draw(&self.report.page_header, Scope { rows: &[], current: None }, false);
        self.y += self.report.page_header.height_mm;
        // Columns start below the page header (and below the report header on page one).
        self.column_top = self.y;
        self.column_bottom = self.y;
    }

    fn finish_page(&mut self) -> Result<()> {
        self.y = self.body_bottom();
        self.draw(&self.report.page_footer, Scope { rows: &[], current: None }, false);
        self.pages.push(std::mem::take(&mut self.page));
        Ok(())
    }

    /// Places a band, moving to a new page first when it doesn't fit, unless the page is fresh
    /// (a break wouldn't give it more room, so it is placed rather than looping).
    fn band_in(&mut self, band: &Band, scope: Scope<'_>, order: &[usize]) -> Result<()> {
        if band.is_empty() {
            return Ok(());
        }
        if self.across() {
            // A group band starts a new row of labels and spans the first column.
            self.close_row();
            if !self.fresh && self.y + band.height_mm > self.body_bottom() + 1e-9 {
                self.new_page(order)?;
            }
            self.draw(band, scope, true);
            self.y += band.height_mm;
            self.column_bottom = self.column_bottom.max(self.y);
            self.fresh = false;
            return Ok(());
        }
        if !self.fresh && self.y + band.height_mm > self.body_bottom() + 1e-9 {
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
        match band.elements.iter().find(|e| matches!(e.content, Content::Subreport { .. })) {
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
        if band.is_empty() {
            return Ok(());
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
        if band.is_empty() {
            return Ok(());
        }
        self.close_row();
        self.y = self.y.max(self.column_bottom);
        self.column = 0;
        if !self.fresh && self.y + band.height_mm > self.body_bottom() + 1e-9 {
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
                self.band_in(footer, Scope { rows: scope, current: last }, order)?;
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
            let text = self.eval(&el.content, el.format.as_ref(), scope);
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
                style: el.style,
                text,
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
            Content::RefreshedAt | Content::PrintedAt => {
                let key = if matches!(content, Content::RefreshedAt) { REFRESHED_AT } else { PRINTED_AT };
                let default = reportcraft_model::Format::Date { pattern: Some(format!("{} %H:%M", self.locale.date_pattern)) };
                self.params
                    .get(key)
                    .map(|t| format::write(&Value::Text(t.clone()), Some(format.unwrap_or(&default)), &self.locale))
                    .unwrap_or_default()
            }
            Content::PageNumber | Content::Line { .. } | Content::Rect { .. } | Content::Barcode { .. } | Content::Subreport { .. } => String::new(),
        }
    }

    fn into_document(mut self) -> Document {
        let total = self.pages.len();
        for (page, item) in std::mem::take(&mut self.page_number_items) {
            if let Some(it) = self.pages.get_mut(page).and_then(|p| p.items.get_mut(item)) {
                it.text = format!("{} / {total}", page + 1);
            }
        }
        let (width_mm, height_mm) = self.report.page.size();
        Document { title: self.report.title.clone(), width_mm, height_mm, pages: self.pages }
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
            keep_together: false,
            repeat_header: false,
            page_break_before: false,
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
            header: Band { height_mm: 6.0, elements: vec![el(Content::Field { field: "branch".into() })] },
            footer: Band::default(),
        });
        r.detail = Band { height_mm: 5.0, elements: vec![el(Content::Field { field: "amount".into() })] };
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
        report.report_header = Band { height_mm: 6.0, elements: vec![el(Content::RefreshedAt), el(Content::PrintedAt)] };
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
