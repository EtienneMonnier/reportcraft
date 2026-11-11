//! ReportCraft Office writers (L4): a laid-out `Document` to XLSX (`rust_xlsxwriter`) and DOCX
//! (`docx-rs`). Both read the document's lines of text: items whose tops line up form a line.
//! XLSX keeps numbers and dates typed with their number format and reads best from a continuous
//! layout (`reportcraft_layout::render_continuous`); DOCX keeps the pages, placing each line at
//! its height and each item on a tab stop at its position. Lines, boxes and barcodes are not
//! written yet: the writers say how many they left out.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod docx;
mod png;
mod xlsx;

pub use docx::to_docx;
pub use xlsx::to_xlsx;

use reportcraft_layout::{Document, Item};

/// A written file and what it couldn't hold.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub bytes: Vec<u8>,
    pub warnings: Vec<String>,
}

/// Items whose tops are this close (in millimetres) are on one line.
const SAME_LINE_MM: f64 = 0.25;

pub(crate) fn mm_to_points(mm: f64) -> f64 {
    mm * 72.0 / 25.4
}

/// The text items of every page as lines, top to bottom and left to right (later pages below
/// earlier ones), and how many drawn items (lines, boxes, barcodes) were left out.
pub(crate) fn lines(doc: &Document) -> (Vec<Vec<&Item>>, usize) {
    let mut lines = Vec::new();
    let mut skipped = 0;
    for page in &doc.pages {
        let (page_lines, page_skipped) = page_lines(&page.items);
        lines.extend(page_lines);
        skipped += page_skipped;
    }
    (lines, skipped)
}

/// One page's text items as lines, and how many drawn items it has.
pub(crate) fn page_lines(items: &[Item]) -> (Vec<Vec<&Item>>, usize) {
    let skipped = items.iter().filter(|it| it.shape.is_some()).count();
    let mut text: Vec<&Item> = items.iter().filter(|it| it.shape.is_none() && !it.text.is_empty()).collect();
    text.sort_by(|a, b| a.y_mm.total_cmp(&b.y_mm).then(a.x_mm.total_cmp(&b.x_mm)));
    let mut lines: Vec<Vec<&Item>> = Vec::new();
    let mut top = f64::NEG_INFINITY;
    for it in text {
        match lines.last_mut() {
            Some(line) if it.y_mm - top <= SAME_LINE_MM => line.push(it),
            _ => {
                top = it.y_mm;
                lines.push(vec![it]);
            }
        }
    }
    for line in &mut lines {
        line.sort_by(|a, b| a.x_mm.total_cmp(&b.x_mm));
    }
    (lines, skipped)
}

pub(crate) fn skipped_warning(skipped: usize, format: &str) -> Vec<String> {
    if skipped == 0 { Vec::new() } else { vec![format!("{skipped} line(s), box(es) or barcode(s) are not written to {format} yet")] }
}
