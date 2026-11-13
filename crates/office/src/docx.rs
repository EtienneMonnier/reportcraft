//! A laid-out report to a DOCX document that keeps its pages: one paragraph per line of text,
//! with an exact height that puts it where the layout did, and a tab stop per item (left, right
//! or centred, as the item is aligned). The result stays editable text in Word or WordCraft.

use docx_rs::{
    AlignmentType, Docx, LineSpacing, LineSpacingType, PageMargin, Paragraph, Pic, RelativeFromHType, RelativeFromVType, Run, RunFonts, Tab,
    TabLeaderType, TabValueType,
};
use reportcraft_core::image::{ImageFormat, Picture};
use reportcraft_core::{Cancel, Error, Result};
use reportcraft_layout::{Document, Item, Shape};
use reportcraft_model::{Align, Color};

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
    let mut pngs = Pngs::new();
    for (n, page) in doc.pages.iter().enumerate() {
        cancel.check("the DOCX export")?;
        let (lines, _) = page_lines(&page.items);
        let (pictures, undrawn) = page_pictures(&page.items, n, &mut pngs);
        skipped += undrawn;
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
        // The page's pictures hang from its first paragraph, placed from the page's corner.
        if let Some(first) = paragraphs.first_mut() {
            for pic in pictures {
                *first = std::mem::take(first).add_run(Run::new().add_image(pic));
            }
        }
        for (i, p) in paragraphs.into_iter().enumerate() {
            docx = docx.add_paragraph(if i == 0 && n > 0 { p.page_break_before(true) } else { p });
        }
    }
    let mut xml = docx.build();
    xml.document = behind_text(&xml.document);
    let mut buffer = std::io::Cursor::new(Vec::new());
    xml.pack(&mut buffer).map_err(|e| Error::Invalid(format!("DOCX: {e}")))?;
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
        // The background is a picture behind the whole box (see `page_pictures`).
        p = p.add_run(run);
    }
    p
}

/// English Metric Units per millimetre (DrawingML positions and sizes).
const EMU_PER_MM: f64 = 36_000.0;

fn emu(mm: f64) -> i32 {
    (mm * EMU_PER_MM).round().clamp(0.0, f64::from(i32::MAX)) as i32
}

/// A picture placed at `x`, `y` (millimetres from the page's corner), `w` × `h` millimetres.
fn placed(png: Vec<u8>, px: (u32, u32), x: f64, y: f64, w: f64, h: f64, id: String) -> Pic {
    let (w, h) = (w.max(0.05), h.max(0.05));
    Pic::new_with_dimensions(png, px.0, px.1)
        .id(id)
        .size(u32::try_from(emu(w)).unwrap_or(1), u32::try_from(emu(h)).unwrap_or(1))
        .floating()
        .overlapping()
        .relative_from_h(RelativeFromHType::Page)
        .relative_from_v(RelativeFromVType::Page)
        .offset_x(emu(x))
        .offset_y(emu(y))
}

/// Image items' pictures as PNG (the only picture format docx-rs writes), by the address of
/// their bytes: a JPEG is converted once however many pages show it; `None` when damaged.
type Pngs = std::collections::HashMap<(usize, usize), Option<std::sync::Arc<Vec<u8>>>>;

fn png_of(shape: &Shape, pngs: &mut Pngs) -> Option<std::sync::Arc<Vec<u8>>> {
    let Shape::Image { format, width_px, height_px, data } = shape else { return None };
    pngs.entry((data.0.as_ptr() as usize, data.0.len()))
        .or_insert_with(|| {
            let picture = Picture { format: *format, width_px: *width_px, height_px: *height_px, data: data.clone() };
            // Decoded whole either way, so a damaged picture never reaches the file.
            let pixels = reportcraft_layout::pixels::decode(&picture)?;
            match format {
                ImageFormat::Png => Some(data.0.to_vec()),
                ImageFormat::Jpeg => reportcraft_layout::pixels::encode_png(&pixels),
            }
            .map(std::sync::Arc::new)
        })
        .clone()
}

/// The pictures drawn behind a page's text: each element's background, lines, boxes,
/// barcodes and images, in the page's order. Also how many shapes couldn't be drawn (diagonal lines).
fn page_pictures(items: &[Item], page: usize, pngs: &mut Pngs) -> (Vec<Pic>, usize) {
    let mut pics = Vec::new();
    let mut undrawn = 0;
    let add = |png: Vec<u8>, px: (u32, u32), x: f64, y: f64, w: f64, h: f64, pics: &mut Vec<Pic>| {
        let id = format!("rIdReportCraft{page}x{}", pics.len());
        pics.push(placed(png, px, x, y, w, h, id));
    };
    let solid = |c: Color| crate::png::solid(c.0, c.1, c.2);
    for it in items {
        let (x, y, w, h) = (it.x_mm, it.y_mm, it.width_mm, it.height_mm);
        if let Some(b) = it.style.background
            && it.shape.is_none()
        {
            add(solid(b), (1, 1), x, y, w, h, &mut pics);
        }
        match &it.shape {
            None => {}
            Some(Shape::Line { thickness_mm, color }) => match Color::parse(color) {
                Some(c) if h.abs() < 1e-9 => add(solid(c), (1, 1), x, y - thickness_mm / 2.0, w, *thickness_mm, &mut pics),
                Some(c) if w.abs() < 1e-9 => add(solid(c), (1, 1), x - thickness_mm / 2.0, y, *thickness_mm, h, &mut pics),
                _ => undrawn += 1,
            },
            Some(Shape::Rect { border_mm, border_color, fill }) => {
                if let Some(f) = fill.as_deref().and_then(Color::parse) {
                    add(solid(f), (1, 1), x, y, w, h, &mut pics);
                }
                match Color::parse(border_color) {
                    Some(c) if *border_mm > 0.0 => {
                        let t = *border_mm;
                        add(solid(c), (1, 1), x - t / 2.0, y - t / 2.0, w + t, t, &mut pics);
                        add(solid(c), (1, 1), x - t / 2.0, y + h - t / 2.0, w + t, t, &mut pics);
                        add(solid(c), (1, 1), x - t / 2.0, y - t / 2.0, t, h + t, &mut pics);
                        add(solid(c), (1, 1), x + w - t / 2.0, y - t / 2.0, t, h + t, &mut pics);
                    }
                    Some(_) => {}
                    None => undrawn += 1,
                }
            }
            Some(Shape::Polygon { points, fill }) => match (Color::parse(fill), polygon_picture(points, x, y, w, h)) {
                (Some(c), Some((px, inside))) => {
                    let png = crate::png::rgba(px.0, px.1, |i, j| inside(i, j).then_some([c.0, c.1, c.2]));
                    add(png, px, x, y, w, h, &mut pics);
                }
                _ => undrawn += 1,
            },
            Some(shape @ Shape::Image { width_px, height_px, .. }) => match png_of(shape, pngs) {
                Some(png) => add(png.to_vec(), (*width_px, *height_px), x, y, w, h, &mut pics),
                None => undrawn += 1,
            },
            Some(Shape::Barcode { columns, rows, quiet, modules }) => match barcode_picture(*columns, *rows, *quiet, modules, w, h) {
                Some(b) => add(b.png, b.pixels, x, y, b.width_mm, b.height_mm, &mut pics),
                None => undrawn += 1,
            },
        }
    }
    (pics, undrawn)
}

/// A barcode as a black-and-white picture with its quiet zone, and the size it takes in the
/// box (a matrix code is square, as large as the box allows), as the PDF writer draws it.
/// Several pixels per module, so a viewer's smoothing never blurs a module away.
/// A polygon's box as pixels (8 per millimetre, at most 1,600 a side) and which pixels it covers.
#[allow(clippy::type_complexity)]
fn polygon_picture(points: &[[f64; 2]], x: f64, y: f64, w: f64, h: f64) -> Option<((u32, u32), Box<dyn Fn(u32, u32) -> bool + '_>)> {
    if points.len() < 3 || !(w > 0.0 && h > 0.0) {
        return None;
    }
    let per_mm = (1_600.0 / w.max(h)).min(8.0);
    let (pw, ph) = ((w * per_mm).ceil().max(1.0) as u32, (h * per_mm).ceil().max(1.0) as u32);
    let inside = move |i: u32, j: u32| {
        // The pixel's centre, in millimetres on the page; even-odd crossing count.
        let (px, py) = (x + (f64::from(i) + 0.5) / per_mm, y + (f64::from(j) + 0.5) / per_mm);
        let mut crossing = false;
        let n = points.len();
        for k in 0..n {
            let ([x1, y1], [x2, y2]) = (points[k], points[(k + 1) % n]);
            if (y1 > py) != (y2 > py) && px < x1 + (py - y1) * (x2 - x1) / (y2 - y1) {
                crossing = !crossing;
            }
        }
        crossing
    };
    Some(((pw, ph), Box::new(inside)))
}

struct BarcodePicture {
    png: Vec<u8>,
    pixels: (u32, u32),
    width_mm: f64,
    height_mm: f64,
}

fn barcode_picture(columns: usize, rows: usize, quiet: usize, modules: &str, w: f64, h: f64) -> Option<BarcodePicture> {
    if columns == 0 || rows == 0 || modules.len() != columns.checked_mul(rows)? || !(w > 0.0 && h > 0.0) {
        return None;
    }
    let linear = rows == 1;
    let span_x = columns + 2 * quiet;
    let span_y = if linear { 1 } else { rows + 2 * quiet };
    let scale = (1_200 / span_x.max(span_y)).clamp(1, 8);
    let (px_w, px_h) = (u32::try_from(span_x * scale).ok()?, if linear { 1 } else { u32::try_from(span_y * scale).ok()? });
    let bytes = modules.as_bytes();
    let png = crate::png::bits(px_w, px_h, |x, y| {
        let mx = (x as usize / scale).checked_sub(quiet);
        let my = if linear { Some(0) } else { (y as usize / scale).checked_sub(quiet) };
        match (mx, my) {
            (Some(mx), Some(my)) if mx < columns && my < rows => bytes.get(my * columns + mx) == Some(&b'1'),
            _ => false,
        }
    });
    let (width_mm, height_mm) = if linear { (w, h) } else { (w.min(h), w.min(h)) };
    Some(BarcodePicture { png, pixels: (px_w, px_h), width_mm, height_mm })
}

/// The page pictures go behind the text and don't push it aside, and every drawing gets its own
/// id (docx-rs writes `behindDoc="0"`, square wrapping and `id="1"` for each).
fn behind_text(document: &[u8]) -> Vec<u8> {
    let xml = String::from_utf8_lossy(document)
        .replace(r#"behindDoc="0""#, r#"behindDoc="1""#)
        .replace(r#"<wp:wrapSquare wrapText="bothSides" />"#, "<wp:wrapNone />");
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml.as_str();
    let mut next = 1;
    while let Some(at) = rest.find(r#"<wp:docPr id=""#) {
        let (before, after) = rest.split_at(at);
        out.push_str(before);
        let skip = after.find('"').map_or(0, |q| q + 1);
        let close = after.get(skip..).and_then(|a| a.find('"')).map_or(skip, |c| skip + c);
        out.push_str(&format!(r#"<wp:docPr id="{next}"#));
        next += 1;
        rest = after.get(close..).unwrap_or_default();
    }
    out.push_str(rest);
    out.into_bytes()
}
