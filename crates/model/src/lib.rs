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
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    #[serde(default = "default_size")]
    pub size_pt: f64,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub align: Align,
}

fn default_size() -> f64 {
    10.0
}

impl Default for TextStyle {
    fn default() -> Self {
        Self { size_pt: default_size(), bold: false, align: Align::Left }
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
    /// "Page N of M".
    PageNumber,
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
        if self.groups.len() > limits::MAX_GROUPS {
            return Err(Error::TooLarge(format!("more than {} groups", limits::MAX_GROUPS)));
        }
        let fixed = self.page_header.height_mm + self.page_footer.height_mm;
        for (name, band) in self.bands() {
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
            for el in &band.elements {
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
        let mut out = Vec::new();
        if let Err(e) = self.validate() {
            out.push(Diagnostic::error("report", e.to_string()));
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
                    Content::Field { .. } | Content::Aggregate { .. } if repeating => {
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
            }],
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
        r.page_header = Band { height_mm: 6.0, elements: vec![el(0.0, 20.0, Content::Field { field: "branch".into() })] };
        r.detail = Band {
            height_mm: 4.0,
            elements: vec![
                el(0.0, 30.0, Content::Field { field: "nope".into() }),
                el(170.0, 20.0, Content::Aggregate { function: AggregateFn::Sum, field: "branch".into() }),
                el(0.0, 10.0, Content::Parameter { name: "period".into() }),
            ],
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
        };
        let mut r = Report::new("t");
        r.locale = "xx-YY".into();
        r.detail =
            Band { height_mm: 5.0, elements: vec![el(Content::Field { field: "name".into() }, Format::Number { decimals: 2, grouping: true })] };
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
        let el =
            |content: Content| Element { x_mm: 0.0, y_mm: 0.0, width_mm: 10.0, height_mm: 0.0, style: TextStyle::default(), content, format: None };
        let mut r = Report::new("t");
        r.detail = Band { height_mm: 5.0, elements: vec![el(Content::Line { thickness_mm: 0.3, color: "#000".into() })] };
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
}
