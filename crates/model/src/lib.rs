//! ReportCraft model (L1): the report definition, a versioned and serde-stable document.
//!
//! A report is banded: a report header, a page header, one header and footer per group, the
//! detail band (once per row), a page footer and a report footer. Positions are in millimetres
//! from the band's top-left corner.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use reportcraft_core::{Error, Result, limits};
use serde::{Deserialize, Serialize};

/// The report file format version this build writes. Every persisted file carries one.
pub const FORMAT_VERSION: u32 = 1;

/// A report definition (`*.rcr.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub format_version: u32,
    pub title: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PageSetup {
    pub width_mm: f64,
    pub height_mm: f64,
    pub margin_mm: f64,
}

impl Default for PageSetup {
    /// A4 portrait, 15 mm margins.
    fn default() -> Self {
        Self { width_mm: 210.0, height_mm: 297.0, margin_mm: 15.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub default: Option<String>,
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
            out.push((format!("group[{i}].header"), &g.header));
        }
        out.push(("detail".to_owned(), &self.detail));
        for (i, g) in self.groups.iter().enumerate().rev() {
            out.push((format!("group[{i}].footer"), &g.footer));
        }
        out.push(("page_footer".to_owned(), &self.page_footer));
        out.push(("report_footer".to_owned(), &self.report_footer));
        out
    }

    /// Rejects geometry and sizes a render could not honour.
    pub fn validate(&self) -> Result<()> {
        let p = &self.page;
        let finite_pos = |v: f64| v.is_finite() && v >= 0.0;
        if !(finite_pos(p.width_mm) && finite_pos(p.height_mm) && finite_pos(p.margin_mm)) {
            return Err(Error::Invalid("page sizes must be finite and not negative".into()));
        }
        if p.width_mm > 5_000.0 || p.height_mm > 5_000.0 {
            return Err(Error::TooLarge("page larger than 5 m".into()));
        }
        let body = p.height_mm - 2.0 * p.margin_mm;
        if p.width_mm - 2.0 * p.margin_mm <= 0.0 || body <= 0.0 {
            return Err(Error::Invalid("margins leave no room on the page".into()));
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
            }],
        };
        let back = Report::from_json(&r.to_json().unwrap()).unwrap();
        assert_eq!(back, r);
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
