//! ReportCraft PDF writer (L4): a laid-out [`Document`] to PDF through craft-libs' `craft-pdf`
//! (Krilla). The page is drawn as the HTML writer draws it: text boxes clipped to their box,
//! aligned with measured widths, lines, rectangles and barcodes as vector paths. Fonts are
//! embedded as subsets; the caller chooses them ([`Fonts`]), so licensing stays its decision.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod fonts;

use craft_pdf::write::backend::SerializeSettings;
use craft_pdf::write::backend::color::rgb;
use craft_pdf::write::backend::geom::{PathBuilder, Point, Rect, Size, Transform};
use craft_pdf::write::backend::image::Image;
use craft_pdf::write::backend::num::NormalizedF32;
use craft_pdf::write::backend::page::PageSettings;
use craft_pdf::write::backend::paint::{Fill, FillRule, Stroke};
use craft_pdf::write::backend::surface::Surface;
use craft_pdf::write::backend::text::{Font, GlyphId, KrillaGlyph};
use craft_pdf::write::{Metadata, WriteError, Writer};
use reportcraft_core::image::ImageFormat;
use reportcraft_core::{Cancel, Error, Result};
use reportcraft_layout::{Document, Item, Page, Shape, color_is_valid};
use reportcraft_model::Align;

pub use fonts::{FontSource, Fonts};

/// Points per millimetre.
const PT_PER_MM: f64 = 72.0 / 25.4;
/// Line height of a text box, in font sizes (the HTML writer's `line-height: 1.2`).
const LINE_HEIGHT: f64 = 1.2;
/// Items drawn between two looks at the cancel flag.
const CANCEL_EVERY: usize = 512;
/// Distinct warnings kept; more are counted in the last one.
const MAX_WARNINGS: usize = 20;

/// A written PDF and what could not be drawn exactly.
#[derive(Debug, Clone)]
pub struct Pdf {
    pub bytes: Vec<u8>,
    /// For example characters the font has no glyph for, or bold drawn with the regular face.
    pub warnings: Vec<String>,
}

/// Writes `doc` as PDF with `fonts`. `producer` names the application and version.
pub fn to_pdf(doc: &Document, fonts: &Fonts, producer: &str, cancel: &Cancel) -> Result<Pdf> {
    let faces = Faces::new(fonts)?;
    let mut warnings = Warnings::default();
    if fonts.bold.is_none() {
        warnings.push("no bold font: bold text is drawn with the regular face".to_owned());
    }
    let bytes = craft_pdf::write::guard(|| {
        let metadata = Metadata {
            creator: "ReportCraft".to_owned(),
            producer: producer.to_owned(),
            title: Some(doc.title.clone()).filter(|t| !t.trim().is_empty()),
            ..Metadata::default()
        };
        let mut writer = Writer::new(SerializeSettings::default(), metadata);
        let (width, height) = ((doc.width_mm * PT_PER_MM) as f32, (doc.height_mm * PT_PER_MM) as f32);
        let settings =
            PageSettings::from_wh(width, height).ok_or_else(|| PdfError::App(Error::Invalid(format!("page size {width} × {height} pt"))))?;
        let mut drawn = 0usize;
        let mut pictures = Pictures::default();
        for page in &doc.pages {
            let mut pdf_page = writer.document_mut().start_page_with(settings.clone());
            let mut surface = pdf_page.surface();
            let mut out = Out { faces: &faces, warnings: &mut warnings, pictures: &mut pictures };
            draw_page(&mut surface, page, &mut out, cancel, &mut drawn).map_err(PdfError::App)?;
            surface.finish();
            pdf_page.finish();
        }
        writer.finish().map_err(PdfError::from)
    })
    .map_err(|e| match e {
        PdfError::App(e) => e,
        PdfError::Write(e) => Error::Unsupported(format!("PDF export failed: {e}")),
    })?;
    Ok(Pdf { bytes, warnings: warnings.finish() })
}

enum PdfError {
    App(Error),
    Write(WriteError),
}

impl From<WriteError> for PdfError {
    fn from(e: WriteError) -> Self {
        PdfError::Write(e)
    }
}

/// One face: Krilla's handle for embedding and rustybuzz's for shaping and metrics.
struct Face<'a> {
    pdf: Font,
    shaper: rustybuzz::Face<'a>,
    /// Ascender and descender over the em, descender negative.
    ascender: f64,
    descender: f64,
}

impl<'a> Face<'a> {
    fn new(bytes: &'a [u8], what: &str) -> Result<Self> {
        let invalid = || Error::Unsupported(format!("the {what} font can't be read (TrueType or OpenType expected)"));
        let shaper = rustybuzz::Face::from_slice(bytes, 0).ok_or_else(invalid)?;
        let pdf = craft_pdf::write::font(bytes, 0, None).map_err(|_| invalid())?;
        let em = f64::from(shaper.units_per_em().max(1));
        let (ascender, descender) = (f64::from(shaper.ascender()) / em, f64::from(shaper.descender()) / em);
        Ok(Self { pdf, shaper, ascender, descender })
    }

    /// Shaped glyphs (advances in ems) and the run's width in ems; the number of characters
    /// the font has no glyph for.
    fn shape(&self, text: &str) -> (Vec<KrillaGlyph>, f64, usize) {
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        let output = rustybuzz::shape(&self.shaper, &[], buffer);
        let em = self.shaper.units_per_em().max(1) as f32;
        let infos = output.glyph_infos();
        let positions = output.glyph_positions();
        let mut glyphs = Vec::with_capacity(infos.len());
        let (mut width, mut missing) = (0.0f64, 0usize);
        for (i, (info, pos)) in infos.iter().zip(positions).enumerate() {
            let start = info.cluster as usize;
            // The cluster runs to the next glyph's cluster (left-to-right text).
            let end = infos.iter().skip(i + 1).map(|g| g.cluster as usize).find(|&c| c != start).unwrap_or(text.len());
            let (start, end) = (start.min(end), start.max(end));
            if info.glyph_id == 0 {
                missing += 1;
            }
            width += f64::from(pos.x_advance) / f64::from(em);
            glyphs.push(KrillaGlyph {
                glyph_id: GlyphId::new(info.glyph_id),
                text_range: start..end,
                x_advance: pos.x_advance as f32 / em,
                x_offset: pos.x_offset as f32 / em,
                y_offset: pos.y_offset as f32 / em,
                y_advance: pos.y_advance as f32 / em,
                location: None,
            });
        }
        (glyphs, width, missing)
    }
}

struct Faces<'a> {
    regular: Face<'a>,
    bold: Option<Face<'a>>,
}

impl<'a> Faces<'a> {
    fn new(fonts: &'a Fonts) -> Result<Self> {
        let regular = Face::new(&fonts.regular, "regular")?;
        let bold = fonts.bold.as_deref().map(|b| Face::new(b, "bold")).transpose()?;
        Ok(Self { regular, bold })
    }

    fn get(&self, bold: bool) -> &Face<'a> {
        if bold { self.bold.as_ref().unwrap_or(&self.regular) } else { &self.regular }
    }
}

#[derive(Default)]
struct Warnings {
    list: Vec<String>,
    /// Distinct warnings beyond the kept ones (the first `MAX_SEEN` remembered to count once).
    dropped: usize,
    seen: std::collections::HashSet<String>,
}

/// Distinct warnings remembered to avoid counting a repeat twice.
const MAX_SEEN: usize = 10_000;

impl Warnings {
    fn push(&mut self, w: String) {
        if self.seen.contains(&w) {
            return;
        }
        if self.seen.len() < MAX_SEEN {
            self.seen.insert(w.clone());
        }
        if self.list.len() < MAX_WARNINGS { self.list.push(w) } else { self.dropped += 1 }
    }

    fn finish(mut self) -> Vec<String> {
        if self.dropped > 0 {
            self.list.push(format!("… and {} more warning(s)", self.dropped));
        }
        self.list
    }
}

/// What drawing a page needs besides the page: the fonts, the warnings so far and the pictures
/// already embedded.
struct Out<'a, 'f> {
    faces: &'a Faces<'f>,
    warnings: &'a mut Warnings,
    pictures: &'a mut Pictures,
}

/// Pictures embedded so far, by the address of their bytes (shared between the pages that show
/// them): each is checked and handed to Krilla once, which writes it once.
#[derive(Default)]
struct Pictures {
    seen: std::collections::HashMap<(usize, usize), Option<Image>>,
}

impl Pictures {
    /// The picture of an image shape, or `None` (with a warning) when it can't be decoded whole:
    /// Krilla decodes PNGs while writing the file, and one damaged picture would fail it all.
    fn get(&mut self, shape: &Shape, warnings: &mut Warnings) -> Option<Image> {
        let Shape::Image { format, width_px, height_px, data } = shape else { return None };
        let key = (data.0.as_ptr() as usize, data.0.len());
        self.seen
            .entry(key)
            .or_insert_with(|| {
                let picture = reportcraft_core::image::Picture { format: *format, width_px: *width_px, height_px: *height_px, data: data.clone() };
                let image = reportcraft_layout::pixels::decode(&picture).and_then(|_| {
                    let bytes = craft_pdf::write::backend::Data::from(data.0.to_vec());
                    match format {
                        ImageFormat::Png => Image::from_png(bytes, true).ok(),
                        ImageFormat::Jpeg => Image::from_jpeg(bytes, true).ok(),
                    }
                });
                if image.is_none() {
                    warnings.push(format!("a {} picture ({} bytes) is damaged and was left out", format.mime(), data.0.len()));
                }
                image
            })
            .clone()
    }
}

fn draw_page(surface: &mut Surface<'_>, page: &Page, out: &mut Out<'_, '_>, cancel: &Cancel, drawn: &mut usize) -> Result<()> {
    // Shapes first, under the text, as in the HTML writer.
    for it in page.items.iter().filter(|i| i.shape.is_some()) {
        tick(cancel, drawn)?;
        match &it.shape {
            Some(shape @ Shape::Image { .. }) => {
                if let Some(image) = out.pictures.get(shape, out.warnings) {
                    draw_image(surface, it, image);
                }
            }
            Some(shape) => draw_shape(surface, it, shape),
            None => {}
        }
    }
    for it in page.items.iter().filter(|i| i.shape.is_none()) {
        tick(cancel, drawn)?;
        draw_text(surface, it, out.faces, out.warnings);
    }
    Ok(())
}

/// A picture stretched over the item's box.
fn draw_image(surface: &mut Surface<'_>, it: &Item, image: Image) {
    let Some(size) = Size::from_wh(mm(it.width_mm), mm(it.height_mm)) else { return };
    surface.push_transform(&Transform::from_translate(mm(it.x_mm), mm(it.y_mm)));
    surface.draw_image(image, size);
    surface.pop();
}

fn tick(cancel: &Cancel, drawn: &mut usize) -> Result<()> {
    *drawn += 1;
    if (*drawn).is_multiple_of(CANCEL_EVERY) { cancel.check("the PDF export") } else { Ok(()) }
}

fn mm(v: f64) -> f32 {
    (v * PT_PER_MM) as f32
}

fn draw_text(surface: &mut Surface<'_>, it: &Item, faces: &Faces<'_>, warnings: &mut Warnings) {
    // The background fills the whole box, text or not.
    if let Some(b) = it.style.background
        && let Some(path) = rect_path(mm(it.x_mm), mm(it.y_mm), mm(it.width_mm), mm(it.height_mm))
    {
        surface.set_fill(Some(fill(rgb::Color::new(b.0, b.1, b.2))));
        surface.set_stroke(None);
        surface.draw_path(&path);
    }
    // Control characters (a line break in a value) print as spaces, as `white-space: nowrap` does.
    let text: String = it.text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    if text.trim().is_empty() || !(it.width_mm > 0.0 && it.height_mm > 0.0 && it.style.size_pt > 0.0) {
        return;
    }
    let face = faces.get(it.style.bold);
    let (glyphs, width_em, missing) = face.shape(&text);
    if missing > 0 {
        warnings.push(format!("the font has no glyph for some characters of “{}”", text.chars().take(40).collect::<String>()));
    }
    let size = it.style.size_pt;
    let (box_x, box_w) = (it.x_mm * PT_PER_MM, it.width_mm * PT_PER_MM);
    let text_w = width_em * size;
    // Text wider than its box starts at the left edge whatever the alignment, as in CSS.
    let x = match it.style.align {
        _ if text_w >= box_w => box_x,
        Align::Left => box_x,
        Align::Center => box_x + (box_w - text_w) / 2.0,
        Align::Right => box_x + box_w - text_w,
    };
    // CSS line box: half the leading above the ascender.
    let content = (face.ascender - face.descender) * size;
    let baseline = it.y_mm * PT_PER_MM + (LINE_HEIGHT * size - content) / 2.0 + face.ascender * size;
    let Some(clip) = rect_path(mm(it.x_mm), mm(it.y_mm), mm(it.width_mm), mm(it.height_mm)) else { return };
    surface.push_clip_path(&clip, &FillRule::NonZero);
    surface.set_fill(Some(it.style.color.map_or_else(Fill::default, |c| fill(rgb::Color::new(c.0, c.1, c.2)))));
    surface.set_stroke(None);
    surface.draw_glyphs(Point::from_xy(x as f32, baseline as f32), &glyphs, face.pdf.clone(), &text, size as f32, false);
    surface.pop();
}

fn rect_path(x: f32, y: f32, w: f32, h: f32) -> Option<craft_pdf::write::backend::geom::Path> {
    let mut b = PathBuilder::new();
    b.push_rect(Rect::from_xywh(x, y, w, h)?);
    b.finish()
}

/// `#rgb` or `#rrggbb` to a colour; `None` for anything else.
fn color(s: &str) -> Option<rgb::Color> {
    if !color_is_valid(s) {
        return None;
    }
    let hex = s.strip_prefix('#')?;
    let digit = |i: usize| hex.get(i..=i).and_then(|d| u8::from_str_radix(d, 16).ok());
    if hex.len() == 3 {
        Some(rgb::Color::new(digit(0)? * 17, digit(1)? * 17, digit(2)? * 17))
    } else {
        let byte = |i: usize| hex.get(i..i + 2).and_then(|d| u8::from_str_radix(d, 16).ok());
        Some(rgb::Color::new(byte(0)?, byte(2)?, byte(4)?))
    }
}

fn fill(c: rgb::Color) -> Fill {
    Fill { paint: c.into(), opacity: NormalizedF32::ONE, rule: FillRule::NonZero }
}

fn stroke(c: rgb::Color, width_pt: f32) -> Stroke {
    Stroke { paint: c.into(), width: width_pt, ..Stroke::default() }
}

fn draw_shape(surface: &mut Surface<'_>, it: &Item, shape: &Shape) {
    match shape {
        Shape::Line { thickness_mm, color: c } => {
            let Some(c) = color(c) else { return };
            let mut b = PathBuilder::new();
            b.move_to(mm(it.x_mm), mm(it.y_mm));
            b.line_to(mm(it.x_mm + it.width_mm), mm(it.y_mm + it.height_mm));
            let Some(path) = b.finish() else { return };
            surface.set_fill(None);
            surface.set_stroke(Some(stroke(c, mm(*thickness_mm))));
            surface.draw_path(&path);
        }
        Shape::Rect { border_mm, border_color, fill: inside } => {
            let Some(border) = color(border_color) else { return };
            let inside = match inside.as_deref() {
                Some(f) => match color(f) {
                    Some(c) => Some(c),
                    None => return,
                },
                None => None,
            };
            let Some(path) = rect_path(mm(it.x_mm), mm(it.y_mm), mm(it.width_mm), mm(it.height_mm)) else { return };
            surface.set_fill(inside.map(fill));
            surface.set_stroke((*border_mm > 0.0).then(|| stroke(border, mm(*border_mm))));
            surface.draw_path(&path);
        }
        Shape::Barcode { columns, rows, quiet, modules } => barcode(surface, it, *columns, *rows, *quiet, modules),
        // Drawn by `draw_image`, which needs the pictures already embedded.
        Shape::Image { .. } => {}
        Shape::Polygon { points, fill: colour } => {
            let Some(c) = color(colour) else { return };
            let mut b = PathBuilder::new();
            let mut corners = points.iter();
            let Some([x, y]) = corners.next() else { return };
            b.move_to(mm(*x), mm(*y));
            for [x, y] in corners {
                b.line_to(mm(*x), mm(*y));
            }
            b.close();
            let Some(path) = b.finish() else { return };
            surface.set_stroke(None);
            surface.set_fill(Some(fill(c)));
            surface.draw_path(&path);
        }
    }
}

/// The HTML writer's barcode geometry: a white ground with the quiet zone, dark runs as rectangles.
fn barcode(surface: &mut Surface<'_>, it: &Item, columns: usize, rows: usize, quiet: usize, modules: &str) {
    if columns == 0 || rows == 0 || modules.len() != columns * rows {
        return;
    }
    let span = (columns + 2 * quiet) as f64;
    let (width, height, sx, sy) = if rows == 1 {
        (it.width_mm, it.height_mm, it.width_mm / span, it.height_mm)
    } else {
        let side = it.width_mm.min(it.height_mm);
        let unit = side / ((rows + 2 * quiet) as f64).max(span);
        (side, side, unit, unit)
    };
    if !(width > 0.0 && height > 0.0) {
        return;
    }
    surface.set_stroke(None);
    if let Some(ground) = rect_path(mm(it.x_mm), mm(it.y_mm), mm(width), mm(height)) {
        surface.set_fill(Some(fill(rgb::Color::new(255, 255, 255))));
        surface.draw_path(&ground);
    }
    let (ox, oy) = (it.x_mm + quiet as f64 * sx, it.y_mm + if rows == 1 { 0.0 } else { quiet as f64 * sy });
    let mut b = PathBuilder::new();
    for (y, row) in modules.as_bytes().chunks(columns).enumerate() {
        let mut x = 0;
        while x < columns {
            if row.get(x) == Some(&b'1') {
                let run = row.iter().skip(x).take_while(|&&c| c == b'1').count();
                if let Some(r) = Rect::from_xywh(mm(ox + x as f64 * sx), mm(oy + y as f64 * sy), mm(run as f64 * sx), mm(sy)) {
                    b.push_rect(r);
                }
                x += run;
            } else {
                x += 1;
            }
        }
    }
    if let Some(path) = b.finish() {
        surface.set_fill(Some(fill(rgb::Color::new(0, 0, 0))));
        surface.draw_path(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_parse_like_the_html_writer_accepts_them() {
        assert_eq!(color("#fff"), Some(rgb::Color::new(255, 255, 255)));
        assert_eq!(color("#1a2B3c"), Some(rgb::Color::new(0x1a, 0x2b, 0x3c)));
        assert_eq!(color("red"), None);
        assert_eq!(color("#12345"), None);
    }

    #[test]
    fn warnings_are_deduplicated_and_capped() {
        let mut w = Warnings::default();
        for i in 0..30 {
            w.push(format!("w{i}"));
            w.push(format!("w{i}"));
        }
        let list = w.finish();
        assert_eq!(list.len(), MAX_WARNINGS + 1);
        assert!(list.last().is_some_and(|l| l.contains("10 more")));
    }
}
