//! Pictures the Office writers place: image items as PNG (sampled down to their print size) and
//! barcodes as black-and-white PNG, shared by DOCX and XLSX.

use reportcraft_core::image::{ImageFormat, Picture};
use reportcraft_layout::{Item, Shape};

/// Image items' pictures as PNG (the only picture format docx-rs writes) with their size in
/// pixels, by the address of their bytes and the pixels their box needs: a JPEG or WebP picture
/// is converted once however many pages show it, and one much larger than its box needs is
/// sampled down to `PRINT_DPI`; `None` when damaged.
pub(crate) type Pngs = std::collections::HashMap<(usize, usize, (u32, u32)), Option<Png>>;

/// A PNG file and its size in pixels.
pub(crate) type Png = (std::sync::Arc<Vec<u8>>, (u32, u32));

pub(crate) fn png_of(it: &Item, pngs: &mut Pngs) -> Option<Png> {
    let Some(Shape::Image { format, width_px, height_px, data }) = &it.shape else { return None };
    let needed = reportcraft_layout::pixels::print_pixels(it.width_mm, it.height_mm);
    pngs.entry((data.0.as_ptr() as usize, data.0.len(), needed))
        .or_insert_with(|| {
            let picture = Picture { format: *format, width_px: *width_px, height_px: *height_px, data: data.clone() };
            // Decoded whole either way, so a damaged picture never reaches the file.
            let pixels = reportcraft_layout::pixels::decode(&picture)?;
            let (png, size) = match (reportcraft_layout::pixels::reduce(&pixels, needed), format) {
                (Some(small), _) => (reportcraft_layout::pixels::encode_png(&small)?, (small.width, small.height)),
                (None, ImageFormat::Png) => (data.0.to_vec(), (pixels.width, pixels.height)),
                (None, ImageFormat::Jpeg | ImageFormat::Webp) => (reportcraft_layout::pixels::encode_png(&pixels)?, (pixels.width, pixels.height)),
            };
            Some((std::sync::Arc::new(png), size))
        })
        .clone()
}

/// A barcode as a black-and-white picture with its quiet zone, and the size it takes in the
/// box (a matrix code is square, as large as the box allows), as the PDF writer draws it.
pub(crate) struct BarcodePicture {
    pub png: Vec<u8>,
    pub pixels: (u32, u32),
    pub width_mm: f64,
    pub height_mm: f64,
}

pub(crate) fn barcode_picture(columns: usize, rows: usize, quiet: usize, modules: &str, w: f64, h: f64) -> Option<BarcodePicture> {
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

/// A picture for an image or barcode item: the PNG and where it goes (millimetres
/// from the page's corner; a matrix code is square inside its box).
pub(crate) struct Placed {
    pub png: std::sync::Arc<Vec<u8>>,
    pub x_mm: f64,
    pub y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
}

/// The picture of an image or barcode item; `None` for other items and damaged pictures.
pub(crate) fn placed(it: &Item, pngs: &mut Pngs) -> Option<Placed> {
    match &it.shape {
        Some(Shape::Image { .. }) => {
            png_of(it, pngs).map(|(png, _)| Placed { png, x_mm: it.x_mm, y_mm: it.y_mm, width_mm: it.width_mm, height_mm: it.height_mm })
        }
        Some(Shape::Barcode { columns, rows, quiet, modules }) => {
            let b = barcode_picture(*columns, *rows, *quiet, modules, it.width_mm, it.height_mm)?;
            Some(Placed { png: std::sync::Arc::new(b.png), x_mm: it.x_mm, y_mm: it.y_mm, width_mm: b.width_mm, height_mm: b.height_mm })
        }
        _ => None,
    }
}
