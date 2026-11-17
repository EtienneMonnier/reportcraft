//! The page wrappers of Odoo's report module, read as page header and footer: a template wrapped
//! in `web.external_layout` gets a letterhead of the document's company (logo, name, address
//! above; phone, e-mail, website and page numbers below), one in `web.internal_layout` a single
//! line (date, company, page numbers).
//!
//! Only what Odoo's documentation says these wrappers show is reproduced, in a layout of our
//! own: the company's fields come from the document (`o.company_id.name`…), so each document's
//! pages carry its own company, which the page bands read from the row their page starts with.

use reportcraft_core::FieldType;
use reportcraft_model::{Align, Band, Content, Element, ImageFit, Report, TextStyle};

use crate::flow::{CONTENT_MM, Field, Level};

/// Which wrapper the template calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Layout {
    External,
    Internal,
}

/// Sets the report's page header and footer for `layout`, the company read through `doc` (the
/// document loop's variable), and records the columns they read.
pub(crate) fn apply(report: &mut Report, layout: Layout, doc: &str, fields: &mut Vec<Field>) {
    let company = |leaf: &str| format!("{doc}.company_id.{leaf}");
    let mut read = |name: String| {
        if !fields.iter().any(|f| f.name.eq_ignore_ascii_case(&name)) {
            fields.push(Field { name: name.clone(), field_type: FieldType::Text, level: Level::Doc, example: None });
        }
        name
    };
    match layout {
        Layout::External => {
            let logo = read(company("logo"));
            let name = read(company("name"));
            let street = read(company("street"));
            let street2 = read(company("street2"));
            let zip = read(company("zip"));
            let city = read(company("city"));
            let country = read(company("country_id.name"));
            let phone = read(company("phone"));
            let email = read(company("email"));
            let website = read(company("website"));
            let right = |size_pt, bold| TextStyle { size_pt, bold, align: Align::Right, ..TextStyle::default() };
            let address = [
                Content::Field { field: street },
                Content::Field { field: street2 },
                Content::Formula { expression: format!("TRIM([{zip}] & \" \" & [{city}])") },
                Content::Field { field: country },
            ];
            let mut header = vec![
                at(0.0, 0.0, 50.0, 18.0, TextStyle::default(), Content::Image { data: None, field: Some(logo), fit: ImageFit::Contain }),
                at(90.0, 0.0, 90.0, 5.5, right(11.0, true), Content::Field { field: name }),
            ];
            header.extend(address.into_iter().zip(0u8..).map(|(c, i)| at(90.0, 6.0 + 4.0 * f64::from(i), 90.0, 4.0, right(8.0, false), c)));
            header.push(rule(23.0));
            report.page_header = Band { height_mm: 26.0, elements: header, ..Band::default() };

            let small = |align| TextStyle { size_pt: 8.0, align, ..TextStyle::default() };
            let footer = vec![
                rule(0.5),
                at(0.0, 1.5, 55.0, 4.0, small(Align::Left), Content::Field { field: phone }),
                at(55.0, 1.5, 70.0, 4.0, small(Align::Center), Content::Field { field: email }),
                at(125.0, 1.5, 55.0, 4.0, small(Align::Right), Content::Field { field: website }),
                at(125.0, 5.5, 55.0, 4.0, small(Align::Right), Content::PageNumber),
            ];
            report.page_footer = Band { height_mm: 10.0, elements: footer, ..Band::default() };
        }
        Layout::Internal => {
            let name = read(company("name"));
            let small = |align| TextStyle { size_pt: 8.0, align, ..TextStyle::default() };
            let header = vec![
                at(0.0, 0.0, 60.0, 4.5, small(Align::Left), Content::Formula { expression: "TEXT(TODAY(), \"yyyy-mm-dd\")".into() }),
                at(60.0, 0.0, 60.0, 4.5, TextStyle { bold: true, ..small(Align::Center) }, Content::Field { field: name }),
                at(120.0, 0.0, 60.0, 4.5, small(Align::Right), Content::PageNumber),
                rule(5.0),
            ];
            report.page_header = Band { height_mm: 7.0, elements: header, ..Band::default() };
        }
    }
}

fn at(x_mm: f64, y_mm: f64, width_mm: f64, height_mm: f64, style: TextStyle, content: Content) -> Element {
    Element { x_mm, y_mm, width_mm, height_mm, style, content, format: None, visible_if: None, color_formula: None, background_formula: None }
}

/// A thin grey rule across the body.
fn rule(y_mm: f64) -> Element {
    at(0.0, y_mm, CONTENT_MM, 0.0, TextStyle::default(), Content::Line { thickness_mm: 0.2, color: "#808080".into() })
}
