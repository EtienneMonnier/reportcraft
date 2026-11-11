//! ReportCraft model (L1): the report definition, a versioned and serde-stable document.
//!
//! A report is banded: a report header, a page header, one header and footer per group, the
//! detail band (once per row), a page footer and a report footer. Positions are in millimetres
//! from the band's top-left corner.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod format;
mod params;

pub use format::{Format, LOCALES, LocaleStyle, MAX_DECIMALS, locale_or_default, locale_style};
pub use params::{MAX_VALUES, ParamType, ParamValue, Parameter, resolve as resolve_parameters};

pub use reportcraft_barcode::{EcLevel, Symbology};
use reportcraft_core::{Column, Error, FieldType, Result, limits};
use serde::{Deserialize, Serialize};

/// The report file format version this build writes. Every persisted file carries one.
pub const FORMAT_VERSION: u32 = 1;

/// A report definition (`*.rcr.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub format_version: u32,
    pub title: String,
    /// How numbers and dates are written (`en`, `en-US`, `fr`, `de-CH`…; unknown tags write as `en`).
    #[serde(default = "default_locale")]
    pub locale: String,
    #[serde(default)]
    pub page: PageSetup,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    /// Computed fields: each row gets a value per formula, usable wherever a field is (elements,
    /// groups, sort, totals, other formulas, subreport links).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formulas: Vec<FormulaField>,
    /// Rows are sorted by these fields before grouping (after the group fields).
    #[serde(default)]
    pub sort: Vec<SortKey>,
    /// Outermost group first.
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub report_header: Band,
    #[serde(default)]
    pub page_header: Band,
    #[serde(default)]
    pub detail: Band,
    #[serde(default)]
    pub page_footer: Band,
    #[serde(default)]
    pub report_footer: Band,
}

/// Whether a `strftime`-style pattern only uses known directives.
pub fn date_pattern_is_valid(pattern: &str) -> bool {
    pattern.len() <= 200 && !chrono::format::StrftimeItems::new(pattern).any(|i| matches!(i, chrono::format::Item::Error))
}

/// A named formula, computed for every row (`"margin": "[price] - [cost]"`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormulaField {
    pub name: String,
    /// Excel-style: `IF([qty] > 10, [price] * 0.9, [price])` (see `reportcraft_expr`).
    pub expression: String,
}

/// Most named formulas one report may declare.
pub const MAX_FORMULAS: usize = 256;

/// Whether a formula name is letters, digits and `_`, not starting with a digit.
fn formula_name_is_valid(name: &str) -> bool {
    name.len() <= 64 && name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') && name.chars().all(|c| c.is_alphanumeric() || c == '_')
}

fn default_locale() -> String {
    "en".to_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PageSetup {
    /// A standard size; when set, it decides the page size and `width_mm`/`height_mm` are ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper: Option<Paper>,
    #[serde(default, skip_serializing_if = "Orientation::is_portrait")]
    pub orientation: Orientation,
    #[serde(default = "a4_width")]
    pub width_mm: f64,
    #[serde(default = "a4_height")]
    pub height_mm: f64,
    pub margin_mm: f64,
    /// Group and detail bands flow down one column, then the next; report and page headers and
    /// footers span the page.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub columns: u8,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub column_gap_mm: f64,
    /// `down`: fill a column, then the next (lists). `across`: fill a row of columns with detail
    /// bands, then the next row (labels); group headers and footers start a new row.
    #[serde(default, skip_serializing_if = "ColumnOrder::is_down")]
    pub column_order: ColumnOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColumnOrder {
    #[default]
    Down,
    Across,
}

impl ColumnOrder {
    fn is_down(&self) -> bool {
        *self == ColumnOrder::Down
    }
}

/// Most columns a page may have.
pub const MAX_COLUMNS: u8 = 16;

fn a4_width() -> f64 {
    210.0
}
fn a4_height() -> f64 {
    297.0
}
fn one() -> u8 {
    1
}
fn is_one(n: &u8) -> bool {
    *n == 1
}
fn is_zero(n: &f64) -> bool {
    *n == 0.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Paper {
    A3,
    A4,
    A5,
    Letter,
    Legal,
}

impl Paper {
    /// Portrait width and height in millimetres.
    pub fn size_mm(self) -> (f64, f64) {
        match self {
            Paper::A3 => (297.0, 420.0),
            Paper::A4 => (210.0, 297.0),
            Paper::A5 => (148.0, 210.0),
            Paper::Letter => (215.9, 279.4),
            Paper::Legal => (215.9, 355.6),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}

impl Orientation {
    fn is_portrait(&self) -> bool {
        *self == Orientation::Portrait
    }
}

impl PageSetup {
    /// A standard paper size, 15 mm margins, one column.
    pub fn paper(paper: Paper, orientation: Orientation) -> Self {
        Self { paper: Some(paper), orientation, ..Self::default() }
    }

    /// The page's width and height: the paper's when set, else `width_mm`/`height_mm`; landscape
    /// puts the long side across.
    pub fn size(&self) -> (f64, f64) {
        let (w, h) = self.paper.map_or((self.width_mm, self.height_mm), Paper::size_mm);
        if self.orientation == Orientation::Landscape && w < h { (h, w) } else { (w, h) }
    }

    /// Width between the margins.
    pub fn body_width(&self) -> f64 {
        self.size().0 - 2.0 * self.margin_mm
    }

    /// Width of one column (the body width when there is one column).
    pub fn column_width(&self) -> f64 {
        let n = f64::from(self.columns.max(1));
        (self.body_width() - self.column_gap_mm * (n - 1.0)) / n
    }
}

impl Default for PageSetup {
    /// A4 portrait, 15 mm margins.
    fn default() -> Self {
        Self {
            paper: None,
            orientation: Orientation::Portrait,
            width_mm: a4_width(),
            height_mm: a4_height(),
            margin_mm: 15.0,
            columns: 1,
            column_gap_mm: 0.0,
            column_order: ColumnOrder::Down,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SortKey {
    pub field: String,
    #[serde(default)]
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub field: String,
    /// Starts the group on a new page when it fits on one page but not in the room left.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep_together: bool,
    /// Prints the group's header again at the top of each page the group continues on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub repeat_header: bool,
    /// Starts every group on a new page (except one that already starts a page).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub page_break_before: bool,
    #[serde(default)]
    pub header: Band,
    #[serde(default)]
    pub footer: Band,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Band {
    #[serde(default)]
    pub height_mm: f64,
    #[serde(default)]
    pub elements: Vec<Element>,
    /// A formula: the band is printed only when it is TRUE (or a non-zero number), and takes no
    /// room otherwise. FALSE, 0 and a blank hide it; an error never hides it. Read with the
    /// band's current row and rows, as formula elements are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_if: Option<String>,
    /// A formula: when TRUE (or a non-zero number), the band starts a new page (unless the page
    /// has nothing on it yet). An error or a blank never breaks the page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_break_before_if: Option<String>,
}

impl Band {
    pub fn is_empty(&self) -> bool {
        self.height_mm <= 0.0 && self.elements.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub x_mm: f64,
    pub y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    #[serde(default)]
    pub style: TextStyle,
    pub content: Content,
    /// How the value is written; `None` writes it plainly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
    /// A formula: the element is drawn only when it is TRUE (or a non-zero number); FALSE, 0 and
    /// a blank hide it, an error never does. Its room in the band stays.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_if: Option<String>,
    /// A formula giving the text's colour for this row (`IF([amount] < 0, "#c00", "")`): a
    /// `#rgb` or `#rrggbb` result replaces the style's colour; anything else keeps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_formula: Option<String>,
    /// The same for the background.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_formula: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    #[serde(default = "default_size")]
    pub size_pt: f64,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub align: Align,
    /// The text's colour (black when unset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    /// Filled behind the element's box (none when unset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Color>,
}

/// A colour, written `#rgb` or `#rrggbb` in report files and always `#rrggbb` when saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Color(pub u8, pub u8, pub u8);

impl Color {
    /// `#rgb` or `#rrggbb`; `None` for anything else.
    pub fn parse(text: &str) -> Option<Color> {
        if !color_is_valid(text) {
            return None;
        }
        let hex = text.strip_prefix('#')?;
        let digit = |i: usize| hex.get(i..=i).and_then(|d| u8::from_str_radix(d, 16).ok());
        if hex.len() == 3 {
            Some(Color(digit(0)? * 17, digit(1)? * 17, digit(2)? * 17))
        } else {
            let byte = |i: usize| hex.get(i..i + 2).and_then(|d| u8::from_str_radix(d, 16).ok());
            Some(Color(byte(0)?, byte(2)?, byte(4)?))
        }
    }

    /// `rrggbb`, without the `#` (DOCX, XLSX).
    pub fn hex(self) -> String {
        format!("{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

impl std::fmt::Display for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.hex())
    }
}

impl TryFrom<String> for Color {
    type Error = String;
    fn try_from(text: String) -> std::result::Result<Self, Self::Error> {
        Color::parse(&text).ok_or_else(|| format!("colour `{text}` must be #rgb or #rrggbb"))
    }
}

impl From<Color> for String {
    fn from(c: Color) -> String {
        c.to_string()
    }
}

fn default_size() -> f64 {
    10.0
}

impl Default for TextStyle {
    fn default() -> Self {
        Self { size_pt: default_size(), bold: false, align: Align::Left, color: None, background: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// What an element shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Content {
    /// Fixed text.
    Text { text: String },
    /// The current row's value of a field (the last row of the group in a group footer).
    Field { field: String },
    /// An aggregate over the rows of the enclosing group, or of the whole report outside groups.
    Aggregate { function: AggregateFn, field: String },
    /// A parameter's value.
    Parameter { name: String },
    /// An Excel-style formula over the current row, parameters and the rows the band summarises:
    /// a field given directly to `SUM`, `COUNT`, `AVERAGE`, `MIN`, `MAX`, `COUNTA`, `SUMIF`,
    /// `COUNTIF` or `AVERAGEIF` covers the group's rows in a group header or footer and in the
    /// detail band (the innermost group, or the whole report), and every row in the report
    /// header and footer.
    Formula { expression: String },
    /// "Page N of M".
    PageNumber,
    /// When the report's data was read (from the database, or from the cache), with a date
    /// format or the locale's date and time.
    RefreshedAt,
    /// When the report was rendered.
    PrintedAt,
    /// A straight line from the element's top-left corner to its bottom-right corner: give it
    /// no height for a horizontal rule, no width for a vertical one.
    Line {
        #[serde(default = "thin")]
        thickness_mm: f64,
        #[serde(default = "black")]
        color: String,
    },
    /// A rectangle over the element's box: a border, a fill, or both.
    Rect {
        #[serde(default = "thin")]
        border_mm: f64,
        #[serde(default = "black")]
        border_color: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fill: Option<String>,
    },
    /// A barcode or QR code over the element's box, encoding a field of the current row or a
    /// fixed text. A value the symbology can't hold is written as text instead.
    Barcode {
        symbology: Symbology,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        field: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        /// The value printed under a linear barcode.
        #[serde(default = "yes")]
        show_text: bool,
        /// QR codes only.
        #[serde(default)]
        ec_level: EcLevel,
    },
    /// Another report laid out in this element's place, over a dataset of its own, flowing
    /// across pages below the band's other elements. With `links`, it reads only the rows that
    /// match the current row (an invoice's lines, a customer's rentals).
    Subreport {
        report: Box<Report>,
        /// The named dataset it reads (loaded with `"dataset": name`).
        dataset: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        links: Vec<SubreportLink>,
        /// With no matching rows, show nothing (else its headers and footers, totals at zero).
        #[serde(default = "yes")]
        hide_when_empty: bool,
    },
}

/// A subreport reads the rows whose `child` field equals the current row's `parent` field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubreportLink {
    pub parent: String,
    pub child: String,
}

/// Reports nest at most this deep, the main report included.
pub const MAX_SUBREPORT_DEPTH: usize = 3;

fn yes() -> bool {
    true
}

fn thin() -> f64 {
    0.3
}

fn black() -> String {
    "#000000".to_owned()
}

/// Thickest line or border accepted.
pub const MAX_LINE_MM: f64 = 50.0;

/// Whether a colour is `#rgb` or `#rrggbb` (the only forms accepted, so a colour can never carry
/// anything else into an output).
pub fn color_is_valid(color: &str) -> bool {
    let hex = color.strip_prefix('#').unwrap_or("");
    (hex.len() == 3 || hex.len() == 6) && hex.chars().all(|c| c.is_ascii_hexdigit())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AggregateFn {
    Sum,
    Count,
    Avg,
    Min,
    Max,
}

impl Report {
    /// An empty report on an A4 page.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            title: title.into(),
            locale: default_locale(),
            page: PageSetup::default(),
            parameters: Vec::new(),
            formulas: Vec::new(),
            sort: Vec::new(),
            groups: Vec::new(),
            report_header: Band::default(),
            page_header: Band::default(),
            detail: Band::default(),
            page_footer: Band::default(),
            report_footer: Band::default(),
        }
    }

    /// Reads a report file, migrating older format versions and validating the result.
    pub fn from_json(text: &str) -> Result<Self> {
        let report: Report = serde_json::from_str(text).map_err(|e| Error::Invalid(format!("report file: {e}")))?;
        if report.format_version == 0 || report.format_version > FORMAT_VERSION {
            return Err(Error::Unsupported(format!("report format version {} (this build reads 1 to {FORMAT_VERSION})", report.format_version)));
        }
        report.validate()?;
        Ok(report)
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| Error::Invalid(e.to_string()))
    }

    /// Every band in print order with its name, for checks and tools.
    pub fn bands(&self) -> Vec<(String, &Band)> {
        let mut out = vec![("report_header".to_owned(), &self.report_header), ("page_header".to_owned(), &self.page_header)];
        for (i, g) in self.groups.iter().enumerate() {
            out.push((format!("groups[{i}].header"), &g.header));
        }
        out.push(("detail".to_owned(), &self.detail));
        for (i, g) in self.groups.iter().enumerate().rev() {
            out.push((format!("groups[{i}].footer"), &g.footer));
        }
        out.push(("page_footer".to_owned(), &self.page_footer));
        out.push(("report_footer".to_owned(), &self.report_footer));
        out
    }

    /// Rejects geometry and sizes a render could not honour.
    pub fn validate(&self) -> Result<()> {
        self.validate_at(0)
    }

    fn validate_at(&self, depth: usize) -> Result<()> {
        let p = &self.page;
        let finite_pos = |v: f64| v.is_finite() && v >= 0.0;
        let (width, height) = p.size();
        if !(finite_pos(width) && finite_pos(height) && finite_pos(p.margin_mm) && finite_pos(p.column_gap_mm)) {
            return Err(Error::Invalid("page sizes must be finite and not negative".into()));
        }
        if width > 5_000.0 || height > 5_000.0 {
            return Err(Error::TooLarge("page larger than 5 m".into()));
        }
        let body = height - 2.0 * p.margin_mm;
        if p.body_width() <= 0.0 || body <= 0.0 {
            return Err(Error::Invalid("margins leave no room on the page".into()));
        }
        if p.columns == 0 || p.columns > MAX_COLUMNS {
            return Err(Error::Invalid(format!("columns must be between 1 and {MAX_COLUMNS}")));
        }
        if p.column_width() <= 0.0 {
            return Err(Error::Invalid("the column gap leaves no room for the columns".into()));
        }
        if let Some(problem) = self.parameters.iter().flat_map(Parameter::declaration_problems).next() {
            return Err(Error::Invalid(problem));
        }
        self.validate_formulas()?;
        if self.groups.len() > limits::MAX_GROUPS {
            return Err(Error::TooLarge(format!("more than {} groups", limits::MAX_GROUPS)));
        }
        let fixed = self.page_header.height_mm + self.page_footer.height_mm;
        for (name, band) in self.bands() {
            for (what, c) in [("visible_if", &band.visible_if), ("page_break_before_if", &band.page_break_before_if)] {
                if let Some(c) = c {
                    reportcraft_expr::parse(c).map_err(|e| Error::Invalid(format!("{name}: {what} `{c}`: {e}")))?;
                }
            }
            if !finite_pos(band.height_mm) {
                return Err(Error::Invalid(format!("{name}: height must be finite and not negative")));
            }
            if band.elements.len() > limits::MAX_ELEMENTS_PER_BAND {
                return Err(Error::TooLarge(format!("{name}: more than {} elements", limits::MAX_ELEMENTS_PER_BAND)));
            }
            let repeating = name == "page_header" || name == "page_footer";
            if !repeating && band.height_mm + fixed > body {
                return Err(Error::Invalid(format!("{name}: band is taller than the room between page header and footer")));
            }
            if band.elements.iter().filter(|e| matches!(e.content, Content::Subreport { .. })).count() > 1 {
                return Err(Error::Invalid(format!("{name}: one subreport per band (put the next one in another band)")));
            }
            for el in &band.elements {
                if let Content::Subreport { report, dataset, .. } = &el.content {
                    if depth + 1 >= MAX_SUBREPORT_DEPTH {
                        return Err(Error::TooLarge(format!(
                            "{name}: reports nest at most {MAX_SUBREPORT_DEPTH} deep (the main report, a subreport, one inside it)"
                        )));
                    }
                    if repeating {
                        return Err(Error::Invalid(format!("{name}: page headers and footers can't hold a subreport")));
                    }
                    if self.page.columns > 1 && self.page.column_order == ColumnOrder::Across {
                        return Err(Error::Invalid(format!("{name}: labels (columns across) can't hold a subreport")));
                    }
                    if dataset.trim().is_empty() || !dataset.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
                        return Err(Error::Invalid(format!("{name}: subreport dataset `{dataset}` must be letters, digits, `_` or `-`")));
                    }
                    if report.bands().iter().any(|(_, b)| b.elements.iter().any(|e| matches!(e.content, Content::PageNumber))) {
                        return Err(Error::Invalid(format!("{name}: page numbers belong to the main report, not a subreport")));
                    }
                    report.validate_at(depth + 1).map_err(|e| Error::Invalid(format!("{name}: subreport: {e}")))?;
                }
                let shape_ok = match &el.content {
                    Content::Line { thickness_mm, color } => (0.0..=MAX_LINE_MM).contains(thickness_mm) && color_is_valid(color),
                    Content::Rect { border_mm, border_color, fill } => {
                        (0.0..=MAX_LINE_MM).contains(border_mm) && color_is_valid(border_color) && fill.as_deref().is_none_or(color_is_valid)
                    }
                    _ => true,
                };
                if !shape_ok {
                    return Err(Error::Invalid(format!("{name}: line width must be 0 to {MAX_LINE_MM} mm and colours #rgb or #rrggbb")));
                }
                if let Content::Barcode { symbology, field, text, ec_level, .. } = &el.content {
                    match (field, text) {
                        (Some(_), None) => {}
                        (None, Some(t)) => {
                            reportcraft_barcode::encode(*symbology, t, *ec_level).map_err(|e| Error::Invalid(format!("{name}: barcode: {e}")))?;
                        }
                        _ => return Err(Error::Invalid(format!("{name}: a barcode encodes either a `field` or a fixed `text`"))),
                    }
                }
                if let Content::Formula { expression } = &el.content {
                    reportcraft_expr::parse(expression).map_err(|e| Error::Invalid(format!("{name}: formula `{expression}`: {e}")))?;
                }
                for (what, c) in
                    [("visible_if", &el.visible_if), ("color_formula", &el.color_formula), ("background_formula", &el.background_formula)]
                {
                    if let Some(c) = c {
                        reportcraft_expr::parse(c).map_err(|e| Error::Invalid(format!("{name}: element {what} `{c}`: {e}")))?;
                    }
                }
                if el.format.as_ref().and_then(Format::decimals).is_some_and(|d| d > MAX_DECIMALS) {
                    return Err(Error::Invalid(format!("{name}: at most {MAX_DECIMALS} decimals")));
                }
                if let Some(Format::Date { pattern: Some(p) }) = &el.format
                    && !date_pattern_is_valid(p)
                {
                    return Err(Error::Invalid(format!("{name}: date pattern `{p}` is not valid")));
                }
                if ![el.x_mm, el.y_mm, el.width_mm, el.height_mm].into_iter().all(finite_pos)
                    || !(el.style.size_pt.is_finite() && el.style.size_pt > 0.0 && el.style.size_pt <= 400.0)
                {
                    return Err(Error::Invalid(format!("{name}: element geometry or font size out of range")));
                }
            }
        }
        Ok(())
    }
}

impl Report {
    /// Named formulas: valid, unique names, formulas that parse, computed per row, no cycle.
    fn validate_formulas(&self) -> Result<()> {
        if self.formulas.len() > MAX_FORMULAS {
            return Err(Error::TooLarge(format!("more than {MAX_FORMULAS} formulas")));
        }
        for (i, f) in self.formulas.iter().enumerate() {
            if !formula_name_is_valid(&f.name) {
                return Err(Error::Invalid(format!("formulas[{i}]: name `{}` must be letters, digits or `_`, not starting with a digit", f.name)));
            }
            if self.formulas.iter().take(i).any(|g| g.name.eq_ignore_ascii_case(&f.name)) {
                return Err(Error::Invalid(format!("formulas[{i}]: `{}` is declared twice", f.name)));
            }
            let expr = reportcraft_expr::parse(&f.expression).map_err(|e| Error::Invalid(format!("formulas[{i}] `{}`: {e}", f.name)))?;
            if expr.uses_ranges() {
                return Err(Error::Invalid(format!(
                    "formulas[{i}] `{}`: a named formula is computed for each row; totals such as SUM([field]) go in a formula element",
                    f.name
                )));
            }
        }
        self.formula_order().map(|_| ())
    }

    /// The named formulas in an order where each comes after the formulas it reads; an error
    /// names a formula that depends on itself.
    pub fn formula_order(&self) -> Result<Vec<usize>> {
        let n = self.formulas.len();
        let index = |name: &str| self.formulas.iter().position(|f| f.name.eq_ignore_ascii_case(name));
        let mut deps: Vec<Vec<usize>> = Vec::with_capacity(n);
        for f in &self.formulas {
            let fields =
                reportcraft_expr::parse(&f.expression).map(|e| e.fields().iter().filter_map(|name| index(name)).collect()).unwrap_or_default();
            deps.push(fields);
        }
        let mut order = Vec::with_capacity(n);
        let mut done = vec![false; n];
        while order.len() < n {
            let ready = (0..n).find(|&i| {
                !done.get(i).copied().unwrap_or(true) && deps.get(i).is_some_and(|d| d.iter().all(|&j| done.get(j).copied().unwrap_or(false)))
            });
            let Some(i) = ready else {
                let stuck: Vec<&str> =
                    (0..n).filter(|&i| !done.get(i).copied().unwrap_or(true)).filter_map(|i| self.formulas.get(i)).map(|f| f.name.as_str()).collect();
                return Err(Error::Invalid(format!("formulas depend on each other in a circle: {}", stuck.join(", "))));
            };
            if let Some(d) = done.get_mut(i) {
                *d = true;
            }
            order.push(i);
        }
        Ok(order)
    }
}

/// How serious a diagnostic is: an error stops rendering or gives a wrong result, a warning
/// shows something the author probably didn't intend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// One finding of `Report::check`, with where it is (`detail.elements[2]`, `groups[0]`…).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub location: String,
    pub message: String,
}

impl Diagnostic {
    pub(crate) fn error(location: impl Into<String>, message: impl Into<String>) -> Self {
        Self { severity: Severity::Error, location: location.into(), message: message.into() }
    }

    pub(crate) fn warning(location: impl Into<String>, message: impl Into<String>) -> Self {
        Self { severity: Severity::Warning, location: location.into(), message: message.into() }
    }
}

impl Report {
    /// Every problem found in the definition, and against the data's columns when given.
    /// Unlike `validate`, it doesn't stop at the first problem and also reports warnings.
    pub fn check(&self, columns: Option<&[Column]>) -> Vec<Diagnostic> {
        self.check_with_datasets(columns, &std::collections::BTreeMap::new())
    }

    /// `check`, with the columns of the named datasets subreports read. When the main data is
    /// given, a subreport whose dataset isn't is an error.
    pub fn check_with_datasets(&self, columns: Option<&[Column]>, datasets: &std::collections::BTreeMap<String, Vec<Column>>) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        if let Err(e) = self.validate() {
            out.push(Diagnostic::error("report", e.to_string()));
        }
        out.extend(self.check_inner(columns, datasets));
        out
    }

    /// Everything `check` finds but what `validate` already said.
    fn check_inner(&self, columns: Option<&[Column]>, datasets: &std::collections::BTreeMap<String, Vec<Column>>) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        if let Some(cols) = columns {
            for (i, f) in self.formulas.iter().enumerate() {
                if cols.iter().any(|c| c.name.eq_ignore_ascii_case(&f.name)) {
                    out.push(Diagnostic::error(format!("formulas[{i}]"), format!("formula `{}` has the name of a data field: rename it", f.name)));
                }
            }
        }
        // Named formulas are fields like the data's own (numeric as far as checks go).
        let extended: Option<Vec<Column>> = columns.map(|cols| {
            cols.iter().cloned().chain(self.formulas.iter().map(|f| Column { name: f.name.clone(), field_type: FieldType::Number })).collect()
        });
        let columns = extended.as_deref();
        for (i, f) in self.formulas.iter().enumerate() {
            out.extend(self.check_formula(&format!("formulas[{i}]"), &f.expression, columns, false));
        }
        let column = |name: &str| columns.map(|cols| cols.iter().find(|c| c.name.eq_ignore_ascii_case(name)));
        let unknown = |name: &str| {
            let names: Vec<&str> = columns.unwrap_or_default().iter().map(|c| c.name.as_str()).collect();
            format!("unknown field `{name}` (the data has: {})", names.join(", "))
        };
        for (i, g) in self.groups.iter().enumerate() {
            if column(&g.field) == Some(None) {
                out.push(Diagnostic::error(format!("groups[{i}]"), unknown(&g.field)));
            }
        }
        for (i, s) in self.sort.iter().enumerate() {
            if column(&s.field) == Some(None) {
                out.push(Diagnostic::error(format!("sort[{i}]"), unknown(&s.field)));
            }
        }
        for (i, p) in self.parameters.iter().enumerate() {
            if self.parameters.iter().take(i).any(|q| q.name == p.name) {
                out.push(Diagnostic::warning(format!("parameters[{i}]"), format!("parameter `{}` is declared twice", p.name)));
            }
        }
        let full_width = self.page.body_width();
        let column_width = self.page.column_width();
        let mut elements = 0usize;
        for (band_name, band) in self.bands() {
            let repeating = band_name == "page_header" || band_name == "page_footer";
            let spans_page = repeating || band_name.starts_with("report_");
            let (width, edge) = if spans_page || self.page.columns <= 1 { (full_width, "the right margin") } else { (column_width, "its column") };
            out.extend(self.check_conditions(&band_name, band, columns, repeating));
            for (i, el) in band.elements.iter().enumerate() {
                elements += 1;
                let at = format!("{band_name}.elements[{i}]");
                let right = el.x_mm + el.width_mm;
                if right > width + 1e-9 {
                    out.push(Diagnostic::warning(&at, format!("extends {:.1} mm past {edge}", right - width)));
                }
                if el.y_mm + el.height_mm > band.height_mm + 1e-9 {
                    out.push(Diagnostic::warning(&at, "extends below its band and may overlap the next one"));
                }
                match &el.content {
                    Content::Field { field } | Content::Aggregate { field, .. } if column(field) == Some(None) => {
                        out.push(Diagnostic::error(&at, unknown(field)));
                    }
                    Content::Barcode { field: Some(field), .. } if column(field) == Some(None) => {
                        out.push(Diagnostic::error(&at, unknown(field)));
                    }
                    Content::Field { .. } | Content::Aggregate { .. } | Content::Barcode { field: Some(_), .. } if repeating => {
                        out.push(Diagnostic::warning(&at, "page headers and footers have no current row: this shows nothing"));
                    }
                    Content::Aggregate { function: AggregateFn::Sum | AggregateFn::Avg | AggregateFn::Min | AggregateFn::Max, field } => {
                        if let Some(Some(c)) = column(field)
                            && matches!(c.field_type, FieldType::Text | FieldType::Bool)
                        {
                            out.push(Diagnostic::warning(&at, format!("`{field}` is not numeric: this total shows nothing")));
                        }
                    }
                    Content::Field { field } if el.format.as_ref().is_some_and(Format::is_numeric) => {
                        if let Some(Some(c)) = column(field)
                            && matches!(c.field_type, FieldType::Text | FieldType::Bool)
                        {
                            out.push(Diagnostic::warning(&at, format!("`{field}` is not numeric: the number format won't apply")));
                        }
                    }
                    Content::Formula { expression } => out.extend(self.check_formula(&at, expression, columns, repeating)),
                    Content::Parameter { name } if !self.parameters.iter().any(|p| &p.name == name) => {
                        out.push(Diagnostic::warning(
                            &at,
                            format!("parameter `{name}` is not declared: it shows only a value passed when rendering"),
                        ));
                    }
                    _ => {}
                }
            }
        }
        let repeated: f64 = self.groups.iter().filter(|g| g.repeat_header).map(|g| g.header.height_mm).sum();
        let body = self.page.size().1 - 2.0 * self.page.margin_mm - self.page_header.height_mm - self.page_footer.height_mm;
        if repeated > 0.0 && repeated + self.detail.height_mm > body {
            out.push(Diagnostic::warning("groups", "repeated group headers leave no room for a detail row on a page"));
        }
        if locale_style(&self.locale).is_none() {
            out.push(Diagnostic::warning("locale", format!("unknown locale `{}`: numbers and dates are written as in `en`", self.locale)));
        }
        if elements == 0 {
            out.push(Diagnostic::warning("report", "the report has no elements: every page is blank"));
        }
        out.extend(self.check_subreports(columns, datasets));
        out
    }

    /// A band's `visible_if` and `page_break_before_if`, and its elements' `visible_if`.
    fn check_conditions(&self, band_name: &str, band: &Band, columns: Option<&[Column]>, repeating: bool) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        if let Some(c) = &band.visible_if {
            out.extend(self.check_formula(&format!("{band_name}.visible_if"), c, columns, repeating));
        }
        match &band.page_break_before_if {
            Some(_) if repeating => {
                out.push(Diagnostic::warning(
                    format!("{band_name}.page_break_before_if"),
                    "page headers and footers never break the page: this is ignored",
                ));
            }
            Some(c) => out.extend(self.check_formula(&format!("{band_name}.page_break_before_if"), c, columns, false)),
            None => {}
        }
        for (i, el) in band.elements.iter().enumerate() {
            for (what, c) in [("visible_if", &el.visible_if), ("color_formula", &el.color_formula), ("background_formula", &el.background_formula)] {
                if let Some(c) = c {
                    out.extend(self.check_formula(&format!("{band_name}.elements[{i}].{what}"), c, columns, repeating));
                }
            }
        }
        out
    }

    /// A formula's fields against the data, its parameters against the declarations.
    fn check_formula(&self, at: &str, expression: &str, columns: Option<&[Column]>, repeating: bool) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let expr = match reportcraft_expr::parse(expression) {
            Ok(e) => e,
            Err(e) => return vec![Diagnostic::error(at, format!("formula: {e}"))],
        };
        if let Some(cols) = columns {
            for name in expr.fields() {
                let known = reportcraft_expr::field_candidates(name).any(|n| cols.iter().any(|c| c.name.eq_ignore_ascii_case(n)));
                if !known {
                    let names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
                    out.push(Diagnostic::error(at, format!("formula: unknown field `{name}` (the data has: {})", names.join(", "))));
                }
            }
        }
        for name in expr.params() {
            if !self.parameters.iter().any(|p| p.name == name || name.strip_suffix(".from").or(name.strip_suffix(".to")) == Some(p.name.as_str())) {
                out.push(Diagnostic::warning(
                    at,
                    format!("formula: parameter `{name}` is not declared: PARAM gives a blank unless a value is passed"),
                ));
            }
        }
        if repeating && (expr.uses_row() || expr.uses_ranges()) {
            out.push(Diagnostic::warning(at, "page headers and footers have no current row: fields in this formula are blank"));
        }
        out
    }

    /// Each subreport against its dataset, and its links against both sides.
    fn check_subreports(&self, columns: Option<&[Column]>, datasets: &std::collections::BTreeMap<String, Vec<Column>>) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let has = |cols: &[Column], name: &str| cols.iter().any(|c| c.name.eq_ignore_ascii_case(name));
        for (band_name, band) in self.bands() {
            for (i, el) in band.elements.iter().enumerate() {
                let Content::Subreport { report, dataset, links, .. } = &el.content else { continue };
                let at = format!("{band_name}.elements[{i}]");
                let child = datasets.get(dataset).map(Vec::as_slice);
                if columns.is_some() && child.is_none() {
                    out.push(Diagnostic::error(&at, format!("dataset `{dataset}` is not loaded: load it with \"dataset\": \"{dataset}\"")));
                }
                for (k, link) in links.iter().enumerate() {
                    if columns.is_some_and(|c| !has(c, &link.parent)) {
                        out.push(Diagnostic::error(format!("{at}.links[{k}]"), format!("unknown field `{}` in the main data", link.parent)));
                    }
                    if child.is_some_and(|c| !has(c, &link.child)) {
                        out.push(Diagnostic::error(format!("{at}.links[{k}]"), format!("unknown field `{}` in dataset `{dataset}`", link.child)));
                    }
                }
                for d in report.check_inner(child, datasets) {
                    out.push(Diagnostic { location: format!("{at}.report.{}", d.location), ..d });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let mut r = Report::new("Test");
        r.detail = Band {
            height_mm: 6.0,
            elements: vec![Element {
                x_mm: 0.0,
                y_mm: 0.0,
                width_mm: 40.0,
                height_mm: 5.0,
                style: TextStyle::default(),
                content: Content::Field { field: "name".into() },
                format: Some(Format::Number { decimals: 2, grouping: true }),
                visible_if: None,
                background_formula: None,
                color_formula: None,
            }],
            page_break_before_if: None,
            visible_if: None,
        };
        let back = Report::from_json(&r.to_json().unwrap()).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn check_reports_every_problem_with_its_location() {
        let col = |n: &str, t| Column { name: n.into(), field_type: t };
        let columns = [col("branch", FieldType::Text), col("amount", FieldType::Number)];
        let el = |x: f64, w: f64, content: Content| Element {
            x_mm: x,
            y_mm: 0.0,
            width_mm: w,
            height_mm: 5.0,
            style: TextStyle::default(),
            content,
            format: None,
            visible_if: None,
            background_formula: None,
            color_formula: None,
        };
        let mut r = Report::new("t");
        r.groups.push(Group {
            field: "region".into(),
            keep_together: false,
            repeat_header: false,
            page_break_before: false,
            header: Band::default(),
            footer: Band::default(),
        });
        r.sort.push(SortKey { field: "amount".into(), descending: false });
        r.page_header = Band {
            height_mm: 6.0,
            elements: vec![el(0.0, 20.0, Content::Field { field: "branch".into() })],
            page_break_before_if: None,
            visible_if: None,
        };
        r.detail = Band {
            height_mm: 4.0,
            elements: vec![
                el(0.0, 30.0, Content::Field { field: "nope".into() }),
                el(170.0, 20.0, Content::Aggregate { function: AggregateFn::Sum, field: "branch".into() }),
                el(0.0, 10.0, Content::Parameter { name: "period".into() }),
            ],
            page_break_before_if: None,
            visible_if: None,
        };
        let found = r.check(Some(&columns));
        let at = |loc: &str, sev| found.iter().any(|d| d.location == loc && d.severity == sev);
        assert!(at("groups[0]", Severity::Error));
        assert!(!found.iter().any(|d| d.location == "sort[0]"), "a known sort field is fine");
        assert!(at("page_header.elements[0]", Severity::Warning));
        assert!(at("detail.elements[0]", Severity::Error));
        assert_eq!(found.iter().filter(|d| d.location == "detail.elements[1]").count(), 3, "past the margin, below its band, a text total");
        assert!(at("detail.elements[2]", Severity::Warning));
        assert_eq!(found.iter().filter(|d| d.message.contains("below its band")).count(), 3);
        // Without data, only the definition itself is checked.
        assert!(!r.check(None).iter().any(|d| d.message.contains("unknown field")));
        assert!(Report::new("blank").check(None).iter().any(|d| d.message.contains("blank")));
    }

    #[test]
    fn formats_are_validated_and_checked() {
        let el = |content: Content, format: Format| Element {
            x_mm: 0.0,
            y_mm: 0.0,
            width_mm: 10.0,
            height_mm: 4.0,
            style: TextStyle::default(),
            content,
            format: Some(format),
            visible_if: None,
            background_formula: None,
            color_formula: None,
        };
        let mut r = Report::new("t");
        r.locale = "xx-YY".into();
        r.detail = Band {
            height_mm: 5.0,
            elements: vec![el(Content::Field { field: "name".into() }, Format::Number { decimals: 2, grouping: true })],
            page_break_before_if: None,
            visible_if: None,
        };
        let columns = [Column { name: "name".into(), field_type: FieldType::Text }];
        let found = r.check(Some(&columns));
        assert!(found.iter().any(|d| d.location == "locale" && d.severity == Severity::Warning));
        assert!(found.iter().any(|d| d.message.contains("number format won't apply")));
        r.detail.elements = vec![el(Content::Text { text: "x".into() }, Format::Date { pattern: Some("%Q".into()) })];
        assert!(r.validate().is_err(), "unknown strftime directive");
        r.detail.elements = vec![el(Content::Text { text: "x".into() }, Format::Percent { decimals: 11 })];
        assert!(r.validate().is_err(), "too many decimals");
        assert!(date_pattern_is_valid("%d/%m/%Y %H:%M") && !date_pattern_is_valid("%"));
    }

    #[test]
    fn paper_sizes_orientation_and_columns() {
        let a4l = PageSetup::paper(Paper::A4, Orientation::Landscape);
        assert_eq!(a4l.size(), (297.0, 210.0));
        let custom = PageSetup { width_mm: 100.0, height_mm: 50.0, orientation: Orientation::Landscape, ..PageSetup::default() };
        assert_eq!(custom.size(), (100.0, 50.0), "already landscape");
        let tall = PageSetup { orientation: Orientation::Landscape, ..PageSetup::default() };
        assert_eq!(tall.size(), (297.0, 210.0), "default A4 turned");
        let cols = PageSetup { columns: 3, column_gap_mm: 5.0, ..PageSetup::paper(Paper::A4, Orientation::Portrait) };
        assert!((cols.column_width() - (180.0 - 10.0) / 3.0).abs() < 1e-9);
        let read: PageSetup = serde_json::from_str(r#"{"paper":"Letter","orientation":"landscape","margin_mm":10}"#).unwrap();
        assert_eq!(read.size(), (279.4, 215.9));
        let mut r = Report::new("t");
        r.page.columns = 0;
        assert!(r.validate().is_err());
        r.page.columns = 4;
        r.page.column_gap_mm = 100.0;
        assert!(r.validate().is_err(), "the gap leaves no room");
    }

    #[test]
    fn colours_and_line_widths_are_checked() {
        assert!(color_is_valid("#abc") && color_is_valid("#A1B2C3"));
        for bad in ["abc", "#ab", "#abcd", "#12345g", "red", "#123456\"", ""] {
            assert!(!color_is_valid(bad), "{bad}");
        }
        let el = |content: Content| Element {
            x_mm: 0.0,
            y_mm: 0.0,
            width_mm: 10.0,
            height_mm: 0.0,
            style: TextStyle::default(),
            content,
            format: None,
            visible_if: None,
            background_formula: None,
            color_formula: None,
        };
        let mut r = Report::new("t");
        r.detail = Band {
            height_mm: 5.0,
            elements: vec![el(Content::Line { thickness_mm: 0.3, color: "#000".into() })],
            page_break_before_if: None,
            visible_if: None,
        };
        assert!(r.validate().is_ok());
        r.detail.elements = vec![el(Content::Line { thickness_mm: 0.3, color: "black".into() })];
        assert!(r.validate().is_err());
        r.detail.elements = vec![el(Content::Rect { border_mm: f64::NAN, border_color: "#000".into(), fill: None })];
        assert!(r.validate().is_err());
        r.detail.elements = vec![el(Content::Rect { border_mm: 0.2, border_color: "#000".into(), fill: Some("url(x)".into()) })];
        assert!(r.validate().is_err());
        let read: Content = serde_json::from_str(r#"{"kind":"line"}"#).unwrap();
        assert_eq!(read, Content::Line { thickness_mm: 0.3, color: "#000000".into() });
    }

    #[test]
    fn refuses_newer_format_and_bad_geometry() {
        let mut r = Report::new("x");
        r.format_version = FORMAT_VERSION + 1;
        assert!(matches!(Report::from_json(&r.to_json().unwrap()), Err(Error::Unsupported(_))));
        let mut r = Report::new("x");
        r.detail.height_mm = f64::NAN;
        assert!(r.validate().is_err());
        let mut r = Report::new("x");
        r.detail.height_mm = 1_000.0;
        assert!(r.validate().is_err());
    }

    #[test]
    fn subreports_are_checked_against_their_dataset() {
        let child = |content: Content| {
            let mut c = Report::new("child");
            c.detail = Band {
                height_mm: 5.0,
                elements: vec![Element {
                    x_mm: 0.0,
                    y_mm: 0.0,
                    width_mm: 50.0,
                    height_mm: 5.0,
                    style: TextStyle::default(),
                    content,
                    format: None,
                    visible_if: None,
                    background_formula: None,
                    color_formula: None,
                }],
                page_break_before_if: None,
                visible_if: None,
            };
            c
        };
        let parent = |sub: Report, links: Vec<SubreportLink>, band: &str| {
            let mut r = Report::new("p");
            let el = Element {
                x_mm: 0.0,
                y_mm: 0.0,
                width_mm: 100.0,
                height_mm: 5.0,
                style: TextStyle::default(),
                format: None,
                content: Content::Subreport { report: Box::new(sub), dataset: "lines".into(), links, hide_when_empty: true },
                visible_if: None,
                background_formula: None,
                color_formula: None,
            };
            let b = Band { height_mm: 10.0, elements: vec![el], page_break_before_if: None, visible_if: None };
            if band == "page_header" {
                r.page_header = b
            } else {
                r.detail = b
            }
            r
        };
        let link = vec![SubreportLink { parent: "id".into(), child: "order_id".into() }];
        let ok = parent(child(Content::Field { field: "item".into() }), link.clone(), "detail");
        let main = [Column { name: "id".into(), field_type: FieldType::Integer }];
        let mut sets = std::collections::BTreeMap::new();
        sets.insert(
            "lines".to_owned(),
            vec![Column { name: "order_id".into(), field_type: FieldType::Integer }, Column { name: "item".into(), field_type: FieldType::Text }],
        );
        assert!(ok.check_with_datasets(Some(&main), &sets).iter().all(|d| d.severity != Severity::Error));
        let typo = parent(child(Content::Field { field: "itme".into() }), link.clone(), "detail").check_with_datasets(Some(&main), &sets);
        assert!(typo.iter().any(|d| d.location == "detail.elements[0].report.detail.elements[0]" && d.message.contains("itme")), "{typo:?}");
        let bad_link =
            parent(child(Content::Text { text: "x".into() }), vec![SubreportLink { parent: "nope".into(), child: "order_id".into() }], "detail");
        assert!(bad_link.check_with_datasets(Some(&main), &sets).iter().any(|d| d.location.ends_with("links[0]")));
        assert!(ok.check_with_datasets(Some(&main), &Default::default()).iter().any(|d| d.message.contains("not loaded")));
        assert!(parent(child(Content::Text { text: "x".into() }), vec![], "page_header").validate().is_err(), "not in page headers");
        assert!(parent(child(Content::PageNumber), vec![], "detail").validate().is_err(), "page numbers stay in the main report");
        let deep = parent(parent(parent(child(Content::Text { text: "x".into() }), vec![], "detail"), vec![], "detail"), vec![], "detail");
        assert!(deep.validate().unwrap_err().to_string().contains("nest"));
        assert!(parent(parent(child(Content::Text { text: "x".into() }), vec![], "detail"), vec![], "detail").validate().is_ok(), "two levels");
    }

    #[test]
    fn formulas_are_validated_and_checked() {
        let formula = |name: &str, expression: &str| FormulaField { name: name.into(), expression: expression.into() };
        let mut r = Report::new("t");
        r.formulas = vec![formula("net", "[gross] - [tax]"), formula("gross", "[price] * [qty]"), formula("tax", "[gross] * 0.2")];
        assert_eq!(r.formula_order().unwrap(), [1, 2, 0], "each after what it reads");
        r.formulas.push(formula("a", "[b] + 1"));
        r.formulas.push(formula("b", "[A] + 1"));
        assert!(r.validate().unwrap_err().to_string().contains("circle: a, b"));
        r.formulas.truncate(3);
        r.formulas.push(formula("Net", "1"));
        assert!(r.validate().unwrap_err().to_string().contains("twice"));
        r.formulas.pop();
        r.formulas.push(formula("1st", "1"));
        assert!(r.validate().is_err(), "bad name");
        r.formulas.pop();
        r.formulas.push(formula("share", "[net] / SUM([net])"));
        assert!(r.validate().unwrap_err().to_string().contains("formula element"));
        r.formulas.pop();
        r.formulas.push(formula("broken", "ROUND([net])"));
        assert!(r.validate().unwrap_err().to_string().contains("formulas[3] `broken`: ROUND takes 2 arguments"));
        r.formulas.pop();
        assert!(r.validate().is_ok());

        let el = |expression: &str| Element {
            x_mm: 0.0,
            y_mm: 0.0,
            width_mm: 20.0,
            height_mm: 4.0,
            style: TextStyle::default(),
            content: Content::Formula { expression: expression.into() },
            format: None,
            visible_if: None,
            background_formula: None,
            color_formula: None,
        };
        r.detail = Band {
            height_mm: 5.0,
            elements: vec![el("[net] / SUM([gross]) & PARAM(\"who\")"), el("[qty] + [nope]"), el("1 +")],
            page_break_before_if: None,
            visible_if: None,
        };
        r.page_footer = Band { height_mm: 5.0, elements: vec![el("[price]")], page_break_before_if: None, visible_if: None };
        assert!(r.validate().unwrap_err().to_string().contains("detail: formula `1 +`"));
        let columns = [Column { name: "price".into(), field_type: FieldType::Number }, Column { name: "qty".into(), field_type: FieldType::Integer }];
        let found = r.check(Some(&columns));
        let has = |loc: &str, sev, text: &str| found.iter().any(|d| d.location == loc && d.severity == sev && d.message.contains(text));
        assert!(has("detail.elements[0]", Severity::Warning, "parameter `who`"), "{found:?}");
        assert!(!found.iter().any(|d| d.location == "detail.elements[0]" && d.severity == Severity::Error), "formula names are fields: {found:?}");
        assert!(has("detail.elements[1]", Severity::Error, "unknown field `nope`"));
        assert!(has("detail.elements[2]", Severity::Error, "ends"));
        assert!(has("page_footer.elements[0]", Severity::Warning, "no current row"));
        let clash = [Column { name: "NET".into(), field_type: FieldType::Number }, Column { name: "price".into(), field_type: FieldType::Number }];
        assert!(r.check(Some(&clash)).iter().any(|d| d.location == "formulas[0]" && d.message.contains("name of a data field")));
        let read: Report = serde_json::from_str(&r.to_json().unwrap()).unwrap();
        assert_eq!(read, r);
    }

    #[test]
    fn conditions_are_validated_and_checked() {
        let text = Element {
            x_mm: 0.0,
            y_mm: 0.0,
            width_mm: 20.0,
            height_mm: 4.0,
            style: TextStyle::default(),
            content: Content::Text { text: "x".into() },
            format: None,
            visible_if: Some("[qty] > 1".into()),
            background_formula: None,
            color_formula: None,
        };
        let mut r = Report::new("t");
        r.detail = Band {
            height_mm: 5.0,
            elements: vec![text.clone()],
            visible_if: Some("[nope] > 0".into()),
            page_break_before_if: Some("[qty] = 0".into()),
        };
        r.page_footer =
            Band { height_mm: 5.0, elements: Vec::new(), visible_if: Some("[qty] > 0".into()), page_break_before_if: Some("TRUE".into()) };
        let columns = [Column { name: "qty".into(), field_type: FieldType::Integer }];
        let found = r.check(Some(&columns));
        let has = |loc: &str, sev, text: &str| found.iter().any(|d| d.location == loc && d.severity == sev && d.message.contains(text));
        assert!(has("detail.visible_if", Severity::Error, "unknown field `nope`"), "{found:?}");
        assert!(!found.iter().any(|d| d.location.starts_with("detail.page_break") || d.location.starts_with("detail.elements[0].visible_if")));
        assert!(has("page_footer.visible_if", Severity::Warning, "no current row"));
        assert!(has("page_footer.page_break_before_if", Severity::Warning, "ignored"));
        r.detail.page_break_before_if = Some("[qty] >".into());
        assert!(r.validate().unwrap_err().to_string().contains("detail: page_break_before_if `[qty] >`"));
        r.detail.page_break_before_if = None;
        r.detail.elements = vec![Element { visible_if: Some("IF(".into()), ..text }];
        assert!(r.validate().unwrap_err().to_string().contains("element visible_if"));
        let read: Band = serde_json::from_str(r#"{"height_mm": 5, "visible_if": "[qty] > 1"}"#).unwrap();
        assert_eq!(read.visible_if.as_deref(), Some("[qty] > 1"));
        assert!(!serde_json::to_string(&Band::default()).unwrap().contains("visible_if"), "absent when unset");
    }

    #[test]
    fn colours_read_short_and_long_forms_and_save_long() {
        let style: TextStyle = serde_json::from_str(r##"{"color": "#c00", "background": "#FFEECC"}"##).unwrap();
        assert_eq!(style.color, Some(Color(0xcc, 0, 0)));
        assert_eq!(style.background, Some(Color(0xff, 0xee, 0xcc)));
        assert_eq!(serde_json::to_string(&style.color).unwrap(), r##""#cc0000""##);
        assert!(serde_json::from_str::<TextStyle>(r#"{"color": "red"}"#).is_err());
        assert!(!serde_json::to_string(&TextStyle::default()).unwrap().contains("color"), "absent when unset");
        let mut r = Report::new("t");
        let el = Element {
            x_mm: 0.0,
            y_mm: 0.0,
            width_mm: 20.0,
            height_mm: 4.0,
            style: TextStyle::default(),
            content: Content::Text { text: "x".into() },
            format: None,
            visible_if: None,
            color_formula: Some("IF([amout] < 0, \"#c00\", \"\")".into()),
            background_formula: Some("IF(".into()),
        };
        r.detail = Band { height_mm: 5.0, elements: vec![el.clone()], ..Band::default() };
        assert!(r.validate().unwrap_err().to_string().contains("background_formula"));
        r.detail.elements = vec![Element { background_formula: None, ..el }];
        let columns = [Column { name: "amount".into(), field_type: FieldType::Number }];
        assert!(r.check(Some(&columns)).iter().any(|d| d.location == "detail.elements[0].color_formula" && d.message.contains("amout")));
    }

    #[test]
    fn barcodes_need_one_encodable_value() {
        let read: Content = serde_json::from_str(r#"{"kind":"barcode","symbology":"qr","field":"url"}"#).unwrap();
        assert_eq!(read, Content::Barcode { symbology: Symbology::Qr, field: Some("url".into()), text: None, show_text: true, ec_level: EcLevel::M });
        let with = |content: Content| {
            let mut r = Report::new("x");
            r.detail = Band {
                height_mm: 20.0,
                elements: vec![Element {
                    x_mm: 0.0,
                    y_mm: 0.0,
                    width_mm: 60.0,
                    height_mm: 15.0,
                    style: TextStyle::default(),
                    content,
                    format: None,
                    visible_if: None,
                    background_formula: None,
                    color_formula: None,
                }],
                page_break_before_if: None,
                visible_if: None,
            };
            r
        };
        let fixed = |text: &str| Content::Barcode {
            symbology: Symbology::Ean13,
            field: None,
            text: Some(text.into()),
            show_text: true,
            ec_level: EcLevel::M,
        };
        assert!(with(fixed("400638133393")).validate().is_ok());
        assert!(with(fixed("4006381333932")).validate().unwrap_err().to_string().contains("check digit"));
        let neither = Content::Barcode { symbology: Symbology::Qr, field: None, text: None, show_text: true, ec_level: EcLevel::M };
        assert!(with(neither).validate().is_err());
        let columns = [Column { name: "asset".into(), field_type: FieldType::Text }];
        let unknown = with(read).check(Some(&columns));
        assert!(unknown.iter().any(|d| d.message.contains("unknown field `url`")), "{unknown:?}");
    }
}
