//! A laid-out report to a DOCX document that keeps its pages: one paragraph per line of text,
//! with an exact height that puts it where the layout did, and a tab stop per item (left, right
//! or centred, as the item is aligned). The result stays editable text in Word or WordCraft.

use docx_rs::{
    AlignmentType, Docx, LineSpacing, LineSpacingType, PageMargin, Paragraph, Run, RunFonts, Shading, ShdType, Tab, TabLeaderType, TabValueType,
};
use reportcraft_core::{Cancel, Error, Result};
use reportcraft_layout::{Document, Item};
use reportcraft_model::Align;

use crate::{Output, page_lines, skipped_warning};

/// Twentieths of a point per millimetre.
const TWIPS_PER_MM: f64 = 1440.0 / 25.4;
/// The height of a line of text, as a multiple of its size (as the HTML and PDF writers use).
const LINE_HEIGHT: f64 = 1.2;
/// Millimetres per point.
const MM_PER_PT: f64 = 25.4 / 72.0;
/// Room kept free above the bottom margin, so rounding never pushes a page's last line onto a
/// page of its own.
const BOTTOM_SLACK_MM: f64 = 3.0;

fn twips(mm: f64) -> i32 {
    (mm.max(0.0) * TWIPS_PER_MM).round().min(f64::from(i32::MAX)) as i32
}

/// Writes `doc` as DOCX, a page of the document per page of the report.
pub fn to_docx(doc: &Document, cancel: &Cancel) -> Result<Output> {
    let margin = doc.margin_mm.max(0.0);
    let page_w = u32::try_from(twips(doc.width_mm)).unwrap_or(0);
    let page_h = u32::try_from(twips(doc.height_mm)).unwrap_or(0);
    if page_w == 0 || page_h == 0 {
        return Err(Error::Invalid("DOCX: the page has no size".into()));
    }
    let arial = RunFonts::new().ascii("Arial").hi_ansi("Arial").cs("Arial").east_asia("Arial");
    let mut docx = Docx::new()
        .page_size(page_w, page_h)
        .page_margin(
            PageMargin::new()
                .top(twips(margin))
                .left(twips(margin))
                .right(twips(margin))
                .bottom(twips((margin - BOTTOM_SLACK_MM).max(0.0)))
                .header(0)
                .footer(0),
        )
        .default_fonts(arial)
        .default_size(20);
    let mut skipped = 0;
    for (n, page) in doc.pages.iter().enumerate() {
        cancel.check("the DOCX export")?;
        let (lines, page_skipped) = page_lines(&page.items);
        skipped += page_skipped;
        let mut bottom = margin;
        let mut paragraphs: Vec<Paragraph> = lines
            .iter()
            .map(|line| {
                // The line's text sits at the bottom of its paragraph: the paragraph ends one text
                // line below the items' top.
                let size = line.iter().map(|it| it.style.size_pt).fold(1.0, f64::max);
                let text_height = size * LINE_HEIGHT * MM_PER_PT;
                let top = line.first().map_or(bottom, |it| it.y_mm);
                let end = (top + text_height).max(bottom + text_height);
                let height = end - bottom;
                bottom = end;
                line_paragraph(line, margin, height)
            })
            .collect();
        if paragraphs.is_empty() {
            paragraphs.push(Paragraph::new().line_spacing(exact(1.0)));
        }
        for (i, p) in paragraphs.into_iter().enumerate() {
            docx = docx.add_paragraph(if i == 0 && n > 0 { p.page_break_before(true) } else { p });
        }
    }
    let mut buffer = std::io::Cursor::new(Vec::new());
    docx.build().pack(&mut buffer).map_err(|e| Error::Invalid(format!("DOCX: {e}")))?;
    Ok(Output { bytes: buffer.into_inner(), warnings: skipped_warning(skipped, "DOCX") })
}

/// An exact line height, no space before or after.
fn exact(height_mm: f64) -> LineSpacing {
    LineSpacing::new().line_rule(LineSpacingType::Exact).line(twips(height_mm).max(1)).before(0).after(0)
}

/// Where an item's tab stop is (twips from the left margin) and what kind it is.
fn tab_of(it: &Item, margin: f64) -> (i32, TabValueType) {
    match it.style.align {
        Align::Left => (twips(it.x_mm - margin), TabValueType::Left),
        Align::Center => (twips(it.x_mm + it.width_mm / 2.0 - margin), TabValueType::Center),
        Align::Right => (twips(it.x_mm + it.width_mm - margin), TabValueType::Right),
    }
}

/// One line of text: its items on tab stops, in the order of their stops.
fn line_paragraph(line: &[&Item], margin: f64, height_mm: f64) -> Paragraph {
    let mut stops: Vec<(i32, TabValueType, &Item)> = line
        .iter()
        .map(|it| {
            let (pos, kind) = tab_of(it, margin);
            (pos, kind, *it)
        })
        .collect();
    stops.sort_by_key(|(pos, ..)| *pos);
    let mut p = Paragraph::new().line_spacing(exact(height_mm)).widow_control(false).align(AlignmentType::Left);
    let tabs: Vec<Tab> = stops
        .iter()
        .filter(|(pos, ..)| *pos > 0)
        .map(|(pos, kind, _)| Tab::new().val(*kind).leader(TabLeaderType::None).pos(usize::try_from(*pos).unwrap_or(0)))
        .collect();
    if !tabs.is_empty() {
        p = p.tabs(&tabs);
    }
    for (pos, kind, it) in stops {
        let starts_at_margin = pos <= 0 && kind == TabValueType::Left;
        if !starts_at_margin {
            p = p.add_run(Run::new().add_tab());
        }
        let mut run = Run::new().size((it.style.size_pt * 2.0).round().clamp(2.0, 3_276.0) as usize).add_text(it.text.replace(['\n', '\r'], " "));
        if it.style.bold {
            run = run.bold();
        }
        if let Some(c) = it.style.color {
            run = run.color(c.hex());
        }
        if let Some(b) = it.style.background {
            run = run.shading(Shading::new().shd_type(ShdType::Clear).color("auto").fill(b.hex()));
        }
        p = p.add_run(run);
    }
    p
}
