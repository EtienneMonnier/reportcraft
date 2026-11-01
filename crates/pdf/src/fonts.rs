//! Which fonts a PDF embeds. No font ships with ReportCraft: the caller passes font files, or
//! [`Fonts::find`] looks, in order, at `REPORTCRAFT_FONT` / `REPORTCRAFT_FONT_BOLD`, a
//! storytold/craft-fonts checkout in `CRAFT_FONTS_DIR` (Inter), then common system fonts
//! (Liberation Sans, DejaVu Sans, Noto Sans, Arial).

#[cfg(not(target_arch = "wasm32"))]
use reportcraft_core::{Error, Result};

/// The regular face and, if there is one, the bold face, as TrueType/OpenType bytes.
#[derive(Clone, Default)]
pub struct Fonts {
    pub regular: Vec<u8>,
    pub bold: Option<Vec<u8>>,
    /// Where the fonts came from, for messages.
    pub source: FontSource,
}

impl std::fmt::Debug for Fonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fonts")
            .field("regular_bytes", &self.regular.len())
            .field("bold_bytes", &self.bold.as_ref().map(Vec::len))
            .field("source", &self.source)
            .finish()
    }
}

/// Where [`Fonts`] were found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontSource {
    pub regular: String,
    pub bold: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
/// Font files over this size are refused (craft-pdf's own limit is far higher).
const MAX_FONT_BYTES: u64 = 64 * 1024 * 1024;

#[cfg(not(target_arch = "wasm32"))]
/// System fonts tried in order: (regular, bold).
const SYSTEM: &[(&str, &str)] = &[
    // Fedora, Arch
    ("/usr/share/fonts/liberation-sans-fonts/LiberationSans-Regular.ttf", "/usr/share/fonts/liberation-sans-fonts/LiberationSans-Bold.ttf"),
    ("/usr/share/fonts/liberation-sans/LiberationSans-Regular.ttf", "/usr/share/fonts/liberation-sans/LiberationSans-Bold.ttf"),
    ("/usr/share/fonts/liberation/LiberationSans-Regular.ttf", "/usr/share/fonts/liberation/LiberationSans-Bold.ttf"),
    ("/usr/share/fonts/TTF/LiberationSans-Regular.ttf", "/usr/share/fonts/TTF/LiberationSans-Bold.ttf"),
    // Debian, Ubuntu
    ("/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf", "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf"),
    ("/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf", "/usr/share/fonts/truetype/liberation2/LiberationSans-Bold.ttf"),
    ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"),
    ("/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf", "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans-Bold.ttf"),
    ("/usr/share/fonts/TTF/DejaVuSans.ttf", "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf"),
    ("/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf", "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf"),
    ("/usr/share/fonts/google-noto/NotoSans-Regular.ttf", "/usr/share/fonts/google-noto/NotoSans-Bold.ttf"),
    // macOS
    ("/System/Library/Fonts/Supplemental/Arial.ttf", "/System/Library/Fonts/Supplemental/Arial Bold.ttf"),
    ("/Library/Fonts/Arial.ttf", "/Library/Fonts/Arial Bold.ttf"),
    // Windows
    ("C:\\Windows\\Fonts\\arial.ttf", "C:\\Windows\\Fonts\\arialbd.ttf"),
];

impl Fonts {
    /// Fonts from bytes the caller already holds.
    pub fn from_bytes(regular: Vec<u8>, bold: Option<Vec<u8>>) -> Self {
        let source = FontSource { regular: "given".to_owned(), bold: bold.as_ref().map(|_| "given".to_owned()) };
        Self { regular, bold, source }
    }

    /// Looks for fonts (see the module comment). Fails with what to set when none is found.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn find() -> Result<Self> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        if let Some(regular) = var("REPORTCRAFT_FONT") {
            let bold = var("REPORTCRAFT_FONT_BOLD");
            return Self::load(&regular, bold.as_deref(), true);
        }
        if let Some(dir) = var("CRAFT_FONTS_DIR") {
            let dir = std::path::Path::new(&dir).join("fonts").join("inter");
            let (r, b) = (dir.join("Inter-Regular.ttf"), dir.join("Inter-Bold.ttf"));
            if r.is_file() {
                return Self::load(&r.to_string_lossy(), b.is_file().then(|| b.to_string_lossy().into_owned()).as_deref(), false);
            }
        }
        for (regular, bold) in SYSTEM {
            if std::path::Path::new(regular).is_file() {
                return Self::load(regular, Some(bold), false);
            }
        }
        Err(Error::NotFound(
            "no font for the PDF: set REPORTCRAFT_FONT (and REPORTCRAFT_FONT_BOLD) to a .ttf/.otf file, or install Liberation Sans or DejaVu Sans"
                .to_owned(),
        ))
    }

    /// Reads the regular font and, when it exists, the bold one. A missing bold font given
    /// explicitly is an error; a missing system bold face is not.
    #[cfg(not(target_arch = "wasm32"))]
    fn load(regular: &str, bold: Option<&str>, explicit: bool) -> Result<Self> {
        let read = |path: &str| -> Result<Vec<u8>> {
            let size = std::fs::metadata(path).map_err(|e| Error::Io(format!("font {path}: {e}")))?.len();
            if size > MAX_FONT_BYTES {
                return Err(Error::TooLarge(format!("font {path} is over {} MiB", MAX_FONT_BYTES >> 20)));
            }
            std::fs::read(path).map_err(|e| Error::Io(format!("font {path}: {e}")))
        };
        let regular_bytes = read(regular)?;
        let bold_bytes = match bold {
            Some(b) if explicit || std::path::Path::new(b).is_file() => Some(read(b)?),
            _ => None,
        };
        let source = FontSource { regular: regular.to_owned(), bold: bold_bytes.as_ref().and(bold).map(str::to_owned) };
        Ok(Self { regular: regular_bytes, bold: bold_bytes, source })
    }
}
