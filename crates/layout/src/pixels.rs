//! Pictures decoded to pixels, for the writers and previews that need them (DOCX takes PNG only,
//! the designer paints pixels) and to check a picture whole before a writer embeds it. Pure Rust
//! decoders, wasm-safe; sizes are checked against `limits` before any pixel is allocated.

use std::io::Cursor;

use reportcraft_core::image::{ImageFormat, Picture};
use reportcraft_core::limits::MAX_IMAGE_PIXELS;

/// A picture as 8-bit RGBA pixels, row by row from the top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// The picture's pixels; `None` when it can't be decoded whole (a damaged file).
pub fn decode(picture: &Picture) -> Option<Rgba> {
    if picture.width_px == 0 || picture.height_px == 0 || u64::from(picture.width_px) * u64::from(picture.height_px) > MAX_IMAGE_PIXELS {
        return None;
    }
    let rgba = match picture.format {
        ImageFormat::Png => png_rgba(&picture.data.0)?,
        ImageFormat::Jpeg => jpeg_rgba(&picture.data.0, picture.width_px, picture.height_px)?,
        ImageFormat::Webp => webp_rgba(&picture.data.0)?,
    };
    let expected = usize::try_from(u64::from(rgba.width) * u64::from(rgba.height) * 4).ok()?;
    (rgba.pixels.len() == expected && rgba.width > 0 && rgba.height > 0).then_some(rgba)
}

/// Resolution pictures are printed at: more pixels than this gives a larger file, not a sharper page.
pub const PRINT_DPI: f64 = 300.0;

/// The pixels a `width_mm` × `height_mm` box needs at `PRINT_DPI` (at least 1 a side).
pub fn print_pixels(width_mm: f64, height_mm: f64) -> (u32, u32) {
    let px = |mm: f64| (mm.max(0.0) / 25.4 * PRINT_DPI).ceil().clamp(1.0, f64::from(u32::MAX)) as u32;
    (px(width_mm), px(height_mm))
}

/// The pixels sampled down to `max` (each pixel the average of those it covers), or `None` when
/// they are within twice that already, where sampling down would save little.
pub fn reduce(rgba: &Rgba, max: (u32, u32)) -> Option<Rgba> {
    let (w, h) = (rgba.width, rgba.height);
    if w <= max.0.saturating_mul(2) && h <= max.1.saturating_mul(2) {
        return None;
    }
    let (tw, th) = (w.min(max.0).max(1), h.min(max.1).max(1));
    let (sw, th64, tw64, sh) = (u64::from(w), u64::from(th), u64::from(tw), u64::from(h));
    let mut out = Vec::with_capacity(usize::try_from(tw64 * th64 * 4).ok()?);
    for oy in 0..th64 {
        let (y0, y1) = (oy * sh / th64, ((oy + 1) * sh / th64).max(oy * sh / th64 + 1));
        for ox in 0..tw64 {
            let (x0, x1) = (ox * sw / tw64, ((ox + 1) * sw / tw64).max(ox * sw / tw64 + 1));
            let mut sum = [0u64; 4];
            for y in y0..y1 {
                let row = usize::try_from(y * sw * 4).ok()?;
                let px = rgba.pixels.get(row + usize::try_from(x0 * 4).ok()?..row + usize::try_from(x1 * 4).ok()?)?;
                for p in px.as_chunks::<4>().0 {
                    for (s, v) in sum.iter_mut().zip(p) {
                        *s += u64::from(*v);
                    }
                }
            }
            let n = (y1 - y0) * (x1 - x0);
            out.extend(sum.iter().map(|s| u8::try_from(s / n).unwrap_or(u8::MAX)));
        }
    }
    Some(Rgba { width: tw, height: th, pixels: out })
}

/// The pixels as a compressed PNG file.
pub fn encode_png(rgba: &Rgba) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, rgba.width, rgba.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&rgba.pixels).ok()?;
        writer.finish().ok()?;
    }
    Some(out)
}

fn png_rgba(bytes: &[u8]) -> Option<Rgba> {
    let budget = usize::try_from(MAX_IMAGE_PIXELS.saturating_mul(8)).unwrap_or(usize::MAX);
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), png::Limits { bytes: budget });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let data = buf.get(..info.buffer_size())?;
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return None,
    };
    if info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let pixels = to_rgba(data, channels, info.width, info.height, info.line_size)?;
    Some(Rgba { width: info.width, height: info.height, pixels })
}

/// Rows of `channels`-byte pixels (with `stride` bytes per row) to RGBA.
fn to_rgba(data: &[u8], channels: usize, width: u32, height: u32, stride: usize) -> Option<Vec<u8>> {
    let (w, h) = (usize::try_from(width).ok()?, usize::try_from(height).ok()?);
    let mut out = Vec::with_capacity(w.checked_mul(h)?.checked_mul(4)?);
    for row in data.chunks(stride).take(h) {
        for px in row.get(..w.checked_mul(channels)?)?.chunks_exact(channels) {
            let p = |i: usize| px.get(i).copied().unwrap_or(255);
            out.extend_from_slice(&match channels {
                1 => [p(0), p(0), p(0), 255],
                2 => [p(0), p(0), p(0), p(1)],
                3 => [p(0), p(1), p(2), 255],
                _ => [p(0), p(1), p(2), p(3)],
            });
        }
    }
    Some(out)
}

fn webp_rgba(bytes: &[u8]) -> Option<Rgba> {
    let mut decoder = image_webp::WebPDecoder::new(Cursor::new(bytes)).ok()?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return None;
    }
    let mut buf = vec![0; decoder.output_buffer_size()?];
    // The first frame of an animation.
    decoder.read_image(&mut buf).ok()?;
    let channels = if decoder.has_alpha() { 4 } else { 3 };
    let stride = usize::try_from(width).ok()?.checked_mul(channels)?;
    let pixels = to_rgba(&buf, channels, width, height, stride)?;
    Some(Rgba { width, height, pixels })
}

fn jpeg_rgba(bytes: &[u8], width: u32, height: u32) -> Option<Rgba> {
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;
    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGBA)
        .set_max_width(usize::try_from(width).ok()?)
        .set_max_height(usize::try_from(height).ok()?);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(Cursor::new(bytes), options);
    let pixels = decoder.decode().ok()?;
    let (w, h) = decoder.dimensions()?;
    Some(Rgba { width: u32::try_from(w).ok()?, height: u32::try_from(h).ok()?, pixels })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny JPEG: 4 × 2 pixels, red on the left half and blue on the right, made for these
    /// tests (`magick -size 2x2 xc:red -size 2x2 xc:blue +append -quality 95 x.jpg`).
    pub(crate) const JPEG_4X2: &[u8] = include_bytes!("../tests/pictures/red-blue-4x2.jpg");
    /// The same, as lossless WebP (`… +append -define webp:lossless=true x.webp`).
    const WEBP_4X2: &[u8] = include_bytes!("../tests/pictures/red-blue-4x2.webp");

    #[test]
    fn webp_decodes() {
        let p = Picture::read(WEBP_4X2).unwrap();
        assert_eq!((p.format, p.width_px, p.height_px), (ImageFormat::Webp, 4, 2));
        let px = decode(&p).unwrap();
        assert_eq!(px.pixels.get(..4), Some(&[255, 0, 0, 255][..]));
        assert_eq!(px.pixels.get(12..16), Some(&[0, 0, 255, 255][..]));
        let cut = Picture::read(WEBP_4X2.get(..30).unwrap()).unwrap();
        assert_eq!(decode(&cut), None);
    }

    #[test]
    fn png_round_trips_and_jpeg_decodes() {
        let rgba = Rgba { width: 2, height: 1, pixels: vec![255, 0, 0, 255, 0, 0, 255, 128] };
        let png = encode_png(&rgba).unwrap();
        let picture = Picture::read(&png).unwrap();
        assert_eq!((picture.format, picture.width_px, picture.height_px), (ImageFormat::Png, 2, 1));
        assert_eq!(decode(&picture), Some(rgba));
        let jpeg = Picture::read(JPEG_4X2).unwrap();
        assert_eq!((jpeg.format, jpeg.width_px, jpeg.height_px), (ImageFormat::Jpeg, 4, 2));
        let px = decode(&jpeg).unwrap();
        assert_eq!((px.width, px.height, px.pixels.len()), (4, 2, 32));
        let (left, right) = (px.pixels.get(..4).unwrap(), px.pixels.get(12..16).unwrap());
        assert!(left[0] > 200 && left[2] < 60, "{left:?}");
        assert!(right[2] > 200 && right[0] < 60, "{right:?}");
    }

    #[test]
    fn damaged_pictures_dont_decode() {
        let png = encode_png(&Rgba { width: 3, height: 3, pixels: vec![9; 36] }).unwrap();
        // The header still reads, the pixel data is cut short.
        let cut = Picture::read(png.get(..png.len() - 20).unwrap()).unwrap();
        assert_eq!(decode(&cut), None);
        let mut garbled = JPEG_4X2.to_vec();
        garbled.truncate(JPEG_4X2.len() / 2);
        if let Ok(p) = Picture::read(&garbled) {
            // zune-jpeg may fill in a truncated scan; what matters is no panic and the right size.
            assert!(decode(&p).is_none_or(|d| d.pixels.len() == 32));
        }
    }

    #[test]
    fn large_pictures_are_sampled_down_to_their_print_size() {
        // 30 mm at 300 dpi.
        assert_eq!(print_pixels(30.0, 10.0), (355, 119));
        assert_eq!(print_pixels(0.0, -1.0), (1, 1));
        // 4 × 2 pixels, left half black, right half white, to 2 × 1: each the average of its 2 × 2.
        let px = [0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255].repeat(2);
        let rgba = Rgba { width: 4, height: 2, pixels: px };
        assert_eq!(reduce(&rgba, (1, 1)), Some(Rgba { width: 1, height: 1, pixels: vec![127, 127, 127, 255] }));
        assert_eq!(reduce(&rgba, (2, 1)), None, "within twice the size: kept");
        let big = Rgba { width: 1000, height: 600, pixels: vec![200; 1000 * 600 * 4] };
        let small = reduce(&big, (100, 60)).unwrap();
        assert_eq!((small.width, small.height, small.pixels.len()), (100, 60, 100 * 60 * 4));
        assert!(small.pixels.iter().all(|&v| v == 200));
    }

    #[test]
    fn grey_and_rgb_rows_become_rgba() {
        assert_eq!(to_rgba(&[10, 20, 0], 1, 2, 1, 3), Some(vec![10, 10, 10, 255, 20, 20, 20, 255]));
        assert_eq!(to_rgba(&[1, 2, 3, 4, 5, 6], 3, 1, 2, 3), Some(vec![1, 2, 3, 255, 4, 5, 6, 255]));
        assert_eq!(to_rgba(&[1, 2], 3, 1, 1, 2), None);
    }
}
