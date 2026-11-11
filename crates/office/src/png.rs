//! Minimal PNG writing for the pictures a DOCX page draws behind its text: one-colour fills and
//! black-and-white barcodes. Uncompressed (stored deflate blocks), which keeps the encoder a few
//! dozen lines with nothing to trust; fills are one pixel and barcodes are 1 bit per pixel, so
//! the files stay small.

/// A 1 × 1 picture of one colour, stretched to the box it fills.
pub(crate) fn solid(r: u8, g: u8, b: u8) -> Vec<u8> {
    // One row: filter byte 0, then the pixel.
    encode(1, 1, 2, 8, &[0, r, g, b])
}

/// A black-and-white picture, `width` × `height` pixels, `dark(x, y)` black.
pub(crate) fn bits(width: u32, height: u32, dark: impl Fn(u32, u32) -> bool) -> Vec<u8> {
    let row_bytes = width.div_ceil(8) as usize;
    let mut raw = Vec::with_capacity((row_bytes + 1) * height as usize);
    for y in 0..height {
        raw.push(0);
        for byte in 0..row_bytes {
            let mut v = 0u8;
            for bit in 0..8 {
                let x = byte as u32 * 8 + bit;
                // Grayscale 1 bit: 1 is white.
                if x < width && !dark(x, y) {
                    v |= 0x80 >> bit;
                }
                if x >= width {
                    v |= 0x80 >> bit;
                }
            }
            raw.push(v);
        }
    }
    encode(width, height, 0, 1, &raw)
}

/// A PNG of filtered scanlines `raw`, colour type and bit depth as the PNG standard numbers them.
fn encode(width: u32, height: u32, color_type: u8, depth: u8, raw: &[u8]) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[depth, color_type, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_stored(raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&u32::try_from(data.len()).unwrap_or(u32::MAX).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(out.get(start..).unwrap_or_default());
    out.extend_from_slice(&crc.to_be_bytes());
}

/// `data` as a zlib stream of stored (uncompressed) deflate blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = if data.is_empty() { vec![&[][..]] } else { data.chunks(65_535).collect() };
    let last = blocks.len().saturating_sub(1);
    for (i, block) in blocks.iter().enumerate() {
        out.push(u8::from(i == last));
        let len = u16::try_from(block.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_match_the_standard_examples() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn pictures_are_well_formed() {
        let red = solid(0xcc, 0, 0);
        assert!(red.starts_with(&[0x89, b'P', b'N', b'G']));
        assert!(red.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]), "IEND with its fixed CRC");
        let checker = bits(10, 3, |x, y| (x + y) % 2 == 0);
        // Width and height in IHDR.
        assert_eq!(checker.get(16..24), Some(&[0, 0, 0, 10, 0, 0, 0, 3][..]));
        // A picture larger than one stored block still ends with its Adler-32.
        assert!(bits(2_000, 300, |_, _| false).len() > 65_535);
    }
}
