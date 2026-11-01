//! QR code (ISO/IEC 18004), byte mode: versions 1 to 40, error correction levels L, M, Q, H, the
//! smallest version that holds the data, and the mask with the lowest penalty.

use crate::{EcLevel, Symbol};

/// Error correction codewords per block, by level (L, M, Q, H) and version (index 0 unused).
const ECC_PER_BLOCK: [[u8; 41]; 4] = [
    [
        0, 7, 10, 15, 20, 26, 18, 20, 24, 30, 18, 20, 24, 26, 30, 22, 24, 28, 30, 28, 28, 28, 28, 30, 30, 26, 28, 30, 30, 30, 30, 30, 30, 30, 30, 30,
        30, 30, 30, 30, 30,
    ],
    [
        0, 10, 16, 26, 18, 24, 16, 18, 22, 22, 26, 30, 22, 22, 24, 24, 28, 28, 26, 26, 26, 26, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28,
        28, 28, 28, 28, 28, 28,
    ],
    [
        0, 13, 22, 18, 26, 18, 24, 18, 22, 20, 24, 28, 26, 24, 20, 30, 24, 28, 28, 26, 30, 28, 30, 30, 30, 30, 28, 30, 30, 30, 30, 30, 30, 30, 30,
        30, 30, 30, 30, 30, 30,
    ],
    [
        0, 17, 28, 22, 16, 22, 28, 26, 26, 24, 28, 24, 28, 22, 24, 24, 30, 28, 28, 26, 28, 30, 24, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30,
        30, 30, 30, 30, 30, 30,
    ],
];

/// Error correction blocks, by level (L, M, Q, H) and version (index 0 unused).
const BLOCKS: [[u8; 41]; 4] = [
    [0, 1, 1, 1, 1, 1, 2, 2, 2, 2, 4, 4, 4, 4, 4, 6, 6, 6, 6, 7, 8, 8, 9, 9, 10, 12, 12, 12, 13, 14, 15, 16, 17, 18, 19, 19, 20, 21, 22, 24, 25],
    [
        0, 1, 1, 1, 2, 2, 4, 4, 4, 5, 5, 5, 8, 9, 9, 10, 10, 11, 13, 14, 16, 17, 17, 18, 20, 21, 23, 25, 26, 28, 29, 31, 33, 35, 37, 38, 40, 43, 45,
        47, 49,
    ],
    [
        0, 1, 1, 2, 2, 4, 4, 6, 6, 8, 8, 8, 10, 12, 16, 12, 17, 16, 18, 21, 20, 23, 23, 25, 27, 29, 34, 34, 35, 38, 40, 43, 45, 48, 51, 53, 56, 59,
        62, 65, 68,
    ],
    [
        0, 1, 1, 2, 4, 4, 4, 5, 6, 8, 8, 11, 11, 16, 16, 18, 16, 19, 21, 25, 25, 25, 34, 30, 32, 35, 37, 40, 42, 45, 48, 51, 54, 57, 60, 63, 66, 70,
        74, 77, 81,
    ],
];

fn level_index(ec: EcLevel) -> usize {
    match ec {
        EcLevel::L => 0,
        EcLevel::M => 1,
        EcLevel::Q => 2,
        EcLevel::H => 3,
    }
}

/// The two format bits of a level.
fn level_bits(ec: EcLevel) -> u32 {
    match ec {
        EcLevel::L => 1,
        EcLevel::M => 0,
        EcLevel::Q => 3,
        EcLevel::H => 2,
    }
}

fn table(t: &[[u8; 41]; 4], ec: EcLevel, version: usize) -> usize {
    usize::from(t.get(level_index(ec)).and_then(|row| row.get(version)).copied().unwrap_or(0))
}

/// Modules left for data and error correction once the function patterns are drawn.
fn raw_data_modules(version: usize) -> usize {
    let v = version;
    let mut n = (16 * v + 128) * v + 64;
    if v >= 2 {
        let align = v / 7 + 2;
        n -= (25 * align - 10) * align - 55;
        if v >= 7 {
            n -= 36;
        }
    }
    n
}

/// Data codewords a version holds at a level.
fn data_codewords(version: usize, ec: EcLevel) -> usize {
    (raw_data_modules(version) / 8).saturating_sub(table(&ECC_PER_BLOCK, ec, version) * table(&BLOCKS, ec, version))
}

/// Centre coordinates of the alignment patterns, on both axes.
fn alignment_positions(version: usize) -> Vec<usize> {
    if version < 2 {
        return Vec::new();
    }
    let size = version * 4 + 17;
    let count = version / 7 + 2;
    let step = if version == 32 { 26 } else { (version * 4 + count * 2 + 1) / (count * 2 - 2) * 2 };
    let mut out: Vec<usize> = (0..count - 1).map(|i| size - 7 - i * step).collect();
    out.push(6);
    out.reverse();
    out
}

/// Multiplication in GF(256) modulo x⁸ + x⁴ + x³ + x² + 1.
fn gf_mul(a: u8, b: u8) -> u8 {
    let mut product: u16 = 0;
    for bit in (0..8).rev() {
        product = (product << 1) ^ if product & 0x80 != 0 { 0x11D } else { 0 };
        if b >> bit & 1 == 1 {
            product ^= u16::from(a);
        }
    }
    u8::try_from(product & 0xFF).unwrap_or(0)
}

/// The Reed-Solomon generator of `degree`, highest coefficient (always 1) left out.
fn generator(degree: usize) -> Vec<u8> {
    let mut g = vec![0u8; degree];
    if let Some(last) = g.last_mut() {
        *last = 1;
    }
    let mut root = 1u8;
    for _ in 0..degree {
        for j in 0..degree {
            let next = g.get(j + 1).copied().unwrap_or(0);
            if let Some(c) = g.get_mut(j) {
                *c = gf_mul(*c, root) ^ next;
            }
        }
        root = gf_mul(root, 2);
    }
    g
}

/// The error correction codewords of `data`.
fn remainder(data: &[u8], generator: &[u8]) -> Vec<u8> {
    let mut r = vec![0u8; generator.len()];
    for &d in data {
        let factor = d ^ r.first().copied().unwrap_or(0);
        r.rotate_left(1);
        if let Some(last) = r.last_mut() {
            *last = 0;
        }
        for (c, &g) in r.iter_mut().zip(generator) {
            *c ^= gf_mul(g, factor);
        }
    }
    r
}

/// The module grid and which modules belong to function patterns.
struct Grid {
    size: usize,
    dark: Vec<bool>,
    function: Vec<bool>,
}

impl Grid {
    fn new(size: usize) -> Self {
        Self { size, dark: vec![false; size * size], function: vec![false; size * size] }
    }

    fn get(&self, x: usize, y: usize) -> bool {
        self.dark.get(y * self.size + x).copied().unwrap_or(false)
    }

    /// Draws a function module (ignored outside the grid).
    fn set(&mut self, x: usize, y: usize, dark: bool) {
        if x < self.size && y < self.size {
            let i = y * self.size + x;
            if let (Some(d), Some(f)) = (self.dark.get_mut(i), self.function.get_mut(i)) {
                *d = dark;
                *f = true;
            }
        }
    }

    fn is_function(&self, x: usize, y: usize) -> bool {
        self.function.get(y * self.size + x).copied().unwrap_or(true)
    }

    /// A finder pattern centred on (cx, cy), with its light separator.
    fn finder(&mut self, cx: usize, cy: usize) {
        for dy in -4i32..=4 {
            for dx in -4i32..=4 {
                let (Some(x), Some(y)) = (cx.checked_add_signed(dx as isize), cy.checked_add_signed(dy as isize)) else { continue };
                let ring = dx.abs().max(dy.abs());
                self.set(x, y, ring != 2 && ring != 4);
            }
        }
    }

    fn alignment(&mut self, cx: usize, cy: usize) {
        for dy in -2i32..=2 {
            for dx in -2i32..=2 {
                let (Some(x), Some(y)) = (cx.checked_add_signed(dx as isize), cy.checked_add_signed(dy as isize)) else { continue };
                self.set(x, y, dx.abs().max(dy.abs()) != 1);
            }
        }
    }

    fn function_patterns(&mut self, version: usize) {
        let size = self.size;
        for i in 0..size {
            self.set(6, i, i % 2 == 0);
            self.set(i, 6, i % 2 == 0);
        }
        self.finder(3, 3);
        self.finder(size - 4, 3);
        self.finder(3, size - 4);
        let positions = alignment_positions(version);
        let last = positions.len().saturating_sub(1);
        for (i, &x) in positions.iter().enumerate() {
            for (j, &y) in positions.iter().enumerate() {
                // Not over the three finder patterns.
                if (i == 0 && j == 0) || (i == 0 && j == last) || (i == last && j == 0) {
                    continue;
                }
                self.alignment(x, y);
            }
        }
        // Reserve the format areas now; their bits are drawn with the mask.
        self.format(0);
        if version >= 7 {
            self.version(version);
        }
    }

    /// The 15 format bits: level and mask, BCH-protected, drawn twice.
    fn format(&mut self, bits_in: u32) {
        let size = self.size;
        let bit = |i: u32| bits_in >> i & 1 == 1;
        for i in 0..6 {
            self.set(8, i as usize, bit(i));
        }
        self.set(8, 7, bit(6));
        self.set(8, 8, bit(7));
        self.set(7, 8, bit(8));
        for i in 9..15 {
            self.set(14 - i as usize, 8, bit(i));
        }
        for i in 0..8 {
            self.set(size - 1 - i as usize, 8, bit(i));
        }
        for i in 8..15 {
            self.set(8, size - 15 + i as usize, bit(i));
        }
        self.set(8, size - 8, true);
    }

    /// The 18 version bits (versions 7 and up), drawn twice.
    fn version(&mut self, version: usize) {
        let v = u32::try_from(version).unwrap_or(0);
        let mut rem = v;
        for _ in 0..12 {
            rem = (rem << 1) ^ ((rem >> 11) * 0x1F25);
        }
        let bits = v << 12 | rem;
        for i in 0..18usize {
            let dark = bits >> i & 1 == 1;
            let (a, b) = (self.size - 11 + i % 3, i / 3);
            self.set(a, b, dark);
            self.set(b, a, dark);
        }
    }

    /// Places the codewords in the two-column zigzag, from the bottom-right corner.
    fn place(&mut self, codewords: &[u8]) {
        let size = self.size;
        let total_bits = codewords.len() * 8;
        let mut i = 0usize;
        let mut right = size - 1;
        loop {
            if right == 6 {
                right = 5;
            }
            for vert in 0..size {
                for j in 0..2 {
                    let Some(x) = right.checked_sub(j) else { continue };
                    let upward = (right + 1) & 2 == 0;
                    let y = if upward { size - 1 - vert } else { vert };
                    if !self.is_function(x, y) && i < total_bits {
                        let byte = codewords.get(i / 8).copied().unwrap_or(0);
                        if let Some(d) = self.dark.get_mut(y * size + x) {
                            *d = byte >> (7 - i % 8) & 1 == 1;
                        }
                        i += 1;
                    }
                }
            }
            if right < 2 {
                break;
            }
            right -= 2;
        }
    }

    /// Flips the data modules where mask `mask` says so (applying it twice undoes it).
    fn apply_mask(&mut self, mask: u32) {
        for y in 0..self.size {
            for x in 0..self.size {
                if self.is_function(x, y) {
                    continue;
                }
                let flip = match mask {
                    0 => (x + y) % 2 == 0,
                    1 => y % 2 == 0,
                    2 => x % 3 == 0,
                    3 => (x + y) % 3 == 0,
                    4 => (x / 3 + y / 2) % 2 == 0,
                    5 => x * y % 2 + x * y % 3 == 0,
                    6 => (x * y % 2 + x * y % 3) % 2 == 0,
                    _ => ((x + y) % 2 + x * y % 3) % 2 == 0,
                };
                if flip && let Some(d) = self.dark.get_mut(y * self.size + x) {
                    *d = !*d;
                }
            }
        }
    }

    /// How hard the symbol is to read: long runs, 2×2 blocks, finder look-alikes, imbalance.
    fn penalty(&self) -> usize {
        let n = self.size;
        let mut score = 0;
        for line in 0..n {
            for horizontal in [true, false] {
                let at = |k: usize| if horizontal { self.get(k, line) } else { self.get(line, k) };
                let mut run = 1;
                for k in 1..n {
                    if at(k) == at(k - 1) {
                        run += 1;
                    } else {
                        score += if run >= 5 { run - 2 } else { 0 };
                        run = 1;
                    }
                }
                score += if run >= 5 { run - 2 } else { 0 };
                // 1:1:3:1:1 with four light modules on one side.
                for k in 0..n.saturating_sub(10) {
                    let w: Vec<bool> = (k..k + 11).map(at).collect();
                    let core = [true, false, true, true, true, false, true];
                    if (w[4..] == core && w[..4].iter().all(|d| !d)) || (w[..7] == core && w[7..].iter().all(|d| !d)) {
                        score += 40;
                    }
                }
            }
        }
        for y in 0..n - 1 {
            for x in 0..n - 1 {
                let c = self.get(x, y);
                if c == self.get(x + 1, y) && c == self.get(x, y + 1) && c == self.get(x + 1, y + 1) {
                    score += 3;
                }
            }
        }
        let dark = self.dark.iter().filter(|d| **d).count();
        let total = n * n;
        score + (dark * 20).abs_diff(total * 10) / total * 10
    }
}

/// Bits appended most significant first.
struct Bits(Vec<bool>);

impl Bits {
    fn push(&mut self, value: usize, count: usize) {
        self.0.extend((0..count).rev().map(|i| value >> i & 1 == 1));
    }
}

/// Longest data accepted (version 40, level L, byte mode).
const MAX_BYTES: usize = 2953;

pub fn encode(data: &[u8], ec: EcLevel) -> Result<Symbol, String> {
    if data.is_empty() {
        return Err("a QR code needs a value".into());
    }
    let fits = |v: usize| {
        let count_bits = if v <= 9 { 8 } else { 16 };
        4 + count_bits + data.len() * 8 <= data_codewords(v, ec) * 8
    };
    let Some(version) = (1..=40).find(|&v| fits(v)) else {
        return Err(format!("too long for a QR code at level {ec:?}: {} bytes (at most {MAX_BYTES} at level L)", data.len()));
    };
    let capacity = data_codewords(version, ec);
    let mut bits = Bits(Vec::with_capacity(capacity * 8));
    bits.push(0b0100, 4);
    bits.push(data.len(), if version <= 9 { 8 } else { 16 });
    for &b in data {
        bits.push(usize::from(b), 8);
    }
    let room = capacity * 8 - bits.0.len();
    bits.push(0, room.min(4));
    let pad = (8 - bits.0.len() % 8) % 8;
    bits.push(0, pad);
    let mut codewords: Vec<u8> = bits.0.chunks(8).map(|c| c.iter().fold(0u8, |acc, &b| acc << 1 | u8::from(b))).collect();
    for pad in [0xEC, 0x11].into_iter().cycle() {
        if codewords.len() >= capacity {
            break;
        }
        codewords.push(pad);
    }

    // Split into blocks, add error correction, interleave.
    let blocks = table(&BLOCKS, ec, version);
    let ecc_len = table(&ECC_PER_BLOCK, ec, version);
    let raw = raw_data_modules(version) / 8;
    let short_blocks = blocks - raw % blocks;
    let short_len = raw / blocks;
    let generator = generator(ecc_len);
    let mut split: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(blocks);
    let mut rest = codewords.as_slice();
    for b in 0..blocks {
        let len = short_len - ecc_len + usize::from(b >= short_blocks);
        let (data, tail) = rest.split_at(len.min(rest.len()));
        rest = tail;
        split.push((data.to_vec(), remainder(data, &generator)));
    }
    let mut out = Vec::with_capacity(raw);
    for i in 0..=short_len - ecc_len {
        out.extend(split.iter().filter_map(|(d, _)| d.get(i)));
    }
    for i in 0..ecc_len {
        out.extend(split.iter().filter_map(|(_, e)| e.get(i)));
    }

    let size = version * 4 + 17;
    let mut grid = Grid::new(size);
    grid.function_patterns(version);
    grid.place(&out);
    let format_bits = |mask: u32| {
        let data = level_bits(ec) << 3 | mask;
        let mut rem = data;
        for _ in 0..10 {
            rem = (rem << 1) ^ ((rem >> 9) * 0x537);
        }
        (data << 10 | rem) ^ 0x5412
    };
    let mut best = (usize::MAX, 0);
    for mask in 0..8 {
        grid.apply_mask(mask);
        grid.format(format_bits(mask));
        let p = grid.penalty();
        if p < best.0 {
            best = (p, mask);
        }
        grid.apply_mask(mask);
    }
    grid.apply_mask(best.1);
    grid.format(format_bits(best.1));
    Ok(Symbol { columns: size, rows: size, quiet: 4, modules: grid.dark, text: String::from_utf8_lossy(data).into_owned() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacities_match_the_standard() {
        // Byte-mode capacities published with the standard.
        let bytes = |v: usize, ec: EcLevel| (data_codewords(v, ec) * 8 - 4 - if v <= 9 { 8 } else { 16 }) / 8;
        assert_eq!(bytes(1, EcLevel::L), 17);
        assert_eq!(bytes(1, EcLevel::H), 7);
        assert_eq!(bytes(10, EcLevel::M), 213);
        assert_eq!(bytes(40, EcLevel::L), MAX_BYTES);
        assert_eq!(bytes(40, EcLevel::H), 1273);
        for ec in [EcLevel::L, EcLevel::M, EcLevel::Q, EcLevel::H] {
            for v in 1..=40 {
                let blocks = table(&BLOCKS, ec, v);
                assert!(raw_data_modules(v) / 8 / blocks > table(&ECC_PER_BLOCK, ec, v), "v{v} {ec:?}");
            }
        }
    }

    #[test]
    fn alignment_patterns_follow_the_standard() {
        assert_eq!(alignment_positions(1), Vec::<usize>::new());
        assert_eq!(alignment_positions(2), [6, 18]);
        assert_eq!(alignment_positions(7), [6, 22, 38]);
        assert_eq!(alignment_positions(32), [6, 34, 60, 86, 112, 138]);
        assert_eq!(alignment_positions(40), [6, 30, 58, 86, 114, 142, 170]);
    }

    #[test]
    fn reed_solomon_of_a_known_message() {
        // "HELLO WORLD" at 1-M, from the standard's worked example: data and its 10 ECC codewords.
        let data = [32, 91, 11, 120, 209, 114, 220, 77, 67, 64, 236, 17, 236, 17, 236, 17];
        assert_eq!(remainder(&data, &generator(10)), [196, 35, 39, 119, 235, 215, 231, 226, 93, 23]);
    }

    #[test]
    fn picks_the_smallest_version() {
        assert_eq!(encode(b"https://example.com", EcLevel::M).unwrap().columns, 25, "version 2");
        assert_eq!(encode(&[b'x'; 17], EcLevel::L).unwrap().columns, 21, "version 1 holds 17 bytes at L");
        assert_eq!(encode(&[b'x'; 18], EcLevel::L).unwrap().columns, 25);
        assert_eq!(encode(&[b'x'; MAX_BYTES], EcLevel::L).unwrap().columns, 177);
        assert!(encode(&[b'x'; MAX_BYTES + 1], EcLevel::L).is_err());
        assert!(encode(b"", EcLevel::M).is_err());
    }
}
