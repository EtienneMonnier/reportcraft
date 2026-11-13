//! Pictures in reports: PNG and JPEG files, recognised by their first bytes, with their size in
//! pixels read from their headers (nothing is decoded here). They come from a report file
//! (base64), a text field (base64 or a `data:` URI) or a database blob.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::limits::{MAX_IMAGE_BYTES, MAX_IMAGE_PIXELS};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Png,
    Jpeg,
}

impl ImageFormat {
    pub fn mime(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
        }
    }
}

/// A picture file's bytes, shared between the pages that show it. Written as base64 in JSON.
#[derive(Clone, PartialEq, Eq)]
pub struct ImageData(pub Arc<[u8]>);

impl fmt::Debug for ImageData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ImageData({} bytes)", self.0.len())
    }
}

impl Serialize for ImageData {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&crate::base64::encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for ImageData {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        crate::base64::decode(&text).map(|b| ImageData(b.into())).ok_or_else(|| serde::de::Error::custom("not base64"))
    }
}

/// A PNG or JPEG picture and its size in pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub format: ImageFormat,
    pub width_px: u32,
    pub height_px: u32,
    pub data: ImageData,
}

/// Why bytes can't be shown as a picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureError {
    /// Neither PNG nor JPEG, or a header too damaged to read its size.
    NotAPicture,
    /// Larger than `MAX_IMAGE_BYTES` or `MAX_IMAGE_PIXELS`, or with no pixels.
    TooLarge,
}

impl fmt::Display for PictureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PictureError::NotAPicture => write!(f, "not a PNG or JPEG picture"),
            PictureError::TooLarge => write!(
                f,
                "the picture is larger than {} MB or {} million pixels, or has no pixels",
                MAX_IMAGE_BYTES >> 20,
                MAX_IMAGE_PIXELS / 1_000_000
            ),
        }
    }
}

impl Picture {
    /// The picture in `bytes`, after checking its format and size.
    pub fn read(bytes: &[u8]) -> Result<Picture, PictureError> {
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(PictureError::TooLarge);
        }
        Picture::of(&ImageData(bytes.into()))
    }

    /// The picture in `data`, sharing its bytes (no copy).
    pub fn of(data: &ImageData) -> Result<Picture, PictureError> {
        if data.0.len() > MAX_IMAGE_BYTES {
            return Err(PictureError::TooLarge);
        }
        let (format, (w, h)) = sniff(&data.0).ok_or(PictureError::NotAPicture)?;
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
            return Err(PictureError::TooLarge);
        }
        Ok(Picture { format, width_px: w, height_px: h, data: data.clone() })
    }

    /// The picture in a text: base64, or a `data:` URI with base64 content.
    pub fn from_text(text: &str) -> Result<Picture, PictureError> {
        let text = text.trim();
        let body = match text.strip_prefix("data:") {
            Some(rest) => rest.split_once(";base64,").map(|(_, b)| b).ok_or(PictureError::NotAPicture)?,
            None => text,
        };
        // Base64 takes four characters for three bytes; whitespace may pad it out a little.
        if body.len() / 4 > MAX_IMAGE_BYTES / 3 + MAX_IMAGE_BYTES / 64 {
            return Err(PictureError::TooLarge);
        }
        let bytes = crate::base64::decode(body).ok_or(PictureError::NotAPicture)?;
        Picture::read(&bytes)
    }

    /// The picture as a `data:` URI.
    pub fn data_uri(&self) -> String {
        format!("data:{};base64,{}", self.format.mime(), crate::base64::encode(&self.data.0))
    }
}

/// A database blob as a value: a `data:` URI when it holds a PNG or JPEG picture (so an image
/// element can show it), `None` otherwise (the source keeps its own way of writing bytes).
pub fn blob_value(bytes: &[u8]) -> Option<crate::Value> {
    Picture::read(bytes).ok().map(|p| crate::Value::Text(p.data_uri()))
}

/// The format and size in pixels of a PNG or JPEG file, read from its header.
pub fn sniff(bytes: &[u8]) -> Option<(ImageFormat, (u32, u32))> {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.starts_with(PNG) {
        // The first chunk is IHDR: length, type, then width and height.
        if bytes.get(12..16)? != b"IHDR" {
            return None;
        }
        return Some((ImageFormat::Png, (be32(bytes, 16)?, be32(bytes, 20)?)));
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        return jpeg_size(bytes).map(|s| (ImageFormat::Jpeg, s));
    }
    None
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

/// Walks a JPEG's marker segments up to its frame header (SOF0–SOF15, except DHT, JPG and DAC).
fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2usize;
    loop {
        // Markers may be preceded by any number of 0xFF fill bytes.
        while *b.get(at)? == 0xFF && *b.get(at + 1)? == 0xFF {
            at += 1;
        }
        if *b.get(at)? != 0xFF {
            return None;
        }
        let marker = *b.get(at + 1)?;
        at += 2;
        // Markers with no length: TEM, RST0–7.
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }
        if marker == 0xD9 || marker == 0xDA {
            // End of image, or the scan started before a frame header: damaged.
            return None;
        }
        let length = usize::from(be16(b, at)?);
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let height = be16(b, at + 3)?;
            let width = be16(b, at + 5)?;
            return Some((u32::from(width), u32::from(height)));
        }
        if length < 2 {
            return None;
        }
        at = at.checked_add(length)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNG header claiming `w` × `h` pixels (enough for `sniff`, not a whole file).
    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[8, 2, 0, 0, 0]);
        b
    }

    /// A JPEG start with an APP0 segment, fill bytes and a progressive frame header.
    fn jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x4A, 0x46, 0xFF, 0xFF, 0xC2, 0x00, 0x0B, 0x08];
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&[1, 1, 0x11, 0]);
        b
    }

    #[test]
    fn reads_the_format_and_size_of_png_and_jpeg() {
        assert_eq!(sniff(&png(640, 480)), Some((ImageFormat::Png, (640, 480))));
        assert_eq!(sniff(&jpeg(300, 200)), Some((ImageFormat::Jpeg, (300, 200))));
        // A DHT segment before the frame isn't mistaken for it.
        let mut with_dht = vec![0xFF, 0xD8, 0xFF, 0xC4, 0x00, 0x03, 0x00];
        with_dht.extend_from_slice(jpeg(7, 9).get(2..).unwrap());
        assert_eq!(sniff(&with_dht), Some((ImageFormat::Jpeg, (7, 9))));
    }

    #[test]
    fn damaged_and_foreign_bytes_are_not_pictures() {
        assert_eq!(sniff(b"GIF89a"), None);
        assert_eq!(sniff(&[]), None);
        let p = png(1, 1);
        for cut in 0..p.len() - 5 {
            assert_eq!(sniff(p.get(..cut).unwrap()), None, "cut at {cut}");
        }
        let j = jpeg(1, 1);
        for cut in 0..j.len() - 4 {
            assert_eq!(sniff(j.get(..cut).unwrap()), None, "cut at {cut}");
        }
        // A zero segment length doesn't loop forever.
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x00, 0xFF, 0xE0]), None);
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x08]), None);
    }

    #[test]
    fn pictures_come_from_base64_data_uris_and_blobs_within_limits() {
        let p = Picture::from_text(&crate::base64::encode(&png(2, 3))).unwrap();
        assert_eq!((p.format, p.width_px, p.height_px), (ImageFormat::Png, 2, 3));
        let uri = p.data_uri();
        assert!(uri.starts_with("data:image/png;base64,iVBOR"));
        assert_eq!(Picture::from_text(&format!(" {uri}\n")), Ok(p));
        assert_eq!(Picture::from_text("data:image/png,raw"), Err(PictureError::NotAPicture));
        assert_eq!(Picture::from_text("not base64!"), Err(PictureError::NotAPicture));
        assert_eq!(Picture::read(&png(0, 5)), Err(PictureError::TooLarge));
        assert_eq!(Picture::read(&png(100_000, 100_000)), Err(PictureError::TooLarge));
        assert_eq!(Picture::from_text(&"A".repeat(MAX_IMAGE_BYTES * 2)), Err(PictureError::TooLarge));
        assert_eq!(blob_value(&jpeg(1, 1)), Some(crate::Value::Text(format!("data:image/jpeg;base64,{}", crate::base64::encode(&jpeg(1, 1))))));
        assert_eq!(blob_value(b"\x00\x01"), None);
    }

    #[test]
    fn image_data_is_base64_in_json_and_short_in_debug() {
        let d = ImageData(png(1, 1).into());
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.starts_with("\"iVBOR"));
        assert_eq!(serde_json::from_str::<ImageData>(&json).unwrap(), d);
        assert!(serde_json::from_str::<ImageData>("\"%%\"").is_err());
        assert_eq!(format!("{d:?}"), format!("ImageData({} bytes)", png(1, 1).len()));
    }
}
