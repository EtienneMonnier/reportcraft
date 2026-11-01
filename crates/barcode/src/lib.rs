//! ReportCraft barcodes (L0): Code 128, EAN-13 and QR code encoders, written from the public
//! standards (ISO/IEC 15417, ISO/IEC 15420, ISO/IEC 18004). No dependencies on other ReportCraft
//! crates, no I/O, wasm-safe. An encoder turns a value into a grid of dark and light modules;
//! drawing it at a size is the writers' job.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod code128;
mod ean;
mod qr;

use serde::{Deserialize, Serialize};

/// The kinds of barcode ReportCraft draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Symbology {
    /// Any printable ASCII text (shipping labels, item codes).
    #[default]
    Code128,
    /// 12 digits (the check digit is added) or 13 (the check digit is verified).
    Ean13,
    /// Any text, UTF-8, up to 2,953 bytes at level L.
    Qr,
}

impl Symbology {
    /// One-dimensional: bars that take the box's full height.
    pub fn is_linear(self) -> bool {
        !matches!(self, Symbology::Qr)
    }
}

/// QR error correction: the share of the symbol that may be damaged and still read
/// (about 7 %, 15 %, 25 %, 30 %).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum EcLevel {
    L,
    #[default]
    M,
    Q,
    H,
}

/// An encoded symbol: `rows` × `columns` modules, row-major, `true` for dark. A linear barcode has
/// one row. `quiet` light modules must surround it to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub columns: usize,
    pub rows: usize,
    pub quiet: usize,
    pub modules: Vec<bool>,
    /// The text printed under a linear barcode (with an EAN-13's check digit).
    pub text: String,
}

impl Symbol {
    pub fn is_dark(&self, row: usize, column: usize) -> bool {
        column < self.columns && self.modules.get(row * self.columns + column).copied().unwrap_or(false)
    }

    /// The modules as `0`/`1` text, one line per row.
    pub fn to_text(&self) -> String {
        self.modules
            .chunks(self.columns.max(1))
            .map(|r| r.iter().map(|&d| if d { '1' } else { '0' }).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Encodes `value`, or says why it can't be.
pub fn encode(symbology: Symbology, value: &str, ec: EcLevel) -> Result<Symbol, String> {
    match symbology {
        Symbology::Code128 => code128::encode(value),
        Symbology::Ean13 => ean::encode(value),
        Symbology::Qr => qr::encode(value.as_bytes(), ec),
    }
}

/// Bars and spaces given as module widths, starting with a bar, to modules.
fn widths_to_modules(widths: impl IntoIterator<Item = u8>, out: &mut Vec<bool>) {
    let mut dark = true;
    for w in widths {
        out.extend(std::iter::repeat_n(dark, usize::from(w)));
        dark = !dark;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbologies_read_from_json() {
        let s: Symbology = serde_json::from_str("\"ean13\"").unwrap();
        assert_eq!(s, Symbology::Ean13);
        let e: EcLevel = serde_json::from_str("\"H\"").unwrap();
        assert_eq!(e, EcLevel::H);
        assert!(Symbology::Code128.is_linear() && !Symbology::Qr.is_linear());
    }
}
