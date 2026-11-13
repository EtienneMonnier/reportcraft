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
    };
    let expected = usize::try_from(u64::from(rgba.width) * u64::from(rgba.height) * 4).ok()?;
    (rgba.pixels.len() == expected && rgba.width > 0 && rgba.height > 0).then_some(rgba)
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
    fn grey_and_rgb_rows_become_rgba() {
        assert_eq!(to_rgba(&[10, 20, 0], 1, 2, 1, 3), Some(vec![10, 10, 10, 255, 20, 20, 20, 255]));
        assert_eq!(to_rgba(&[1, 2, 3, 4, 5, 6], 3, 1, 2, 3), Some(vec![1, 2, 3, 255, 4, 5, 6, 255]));
        assert_eq!(to_rgba(&[1, 2], 3, 1, 1, 2), None);
    }
}
