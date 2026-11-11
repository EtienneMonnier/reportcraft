//! Built-in samples, defined in code. The data is invented for ReportCraft.

use reportcraft_model::{AggregateFn, Align, Band, Content, Element, Format, Group, Parameter, Report, SortKey, TextStyle};

/// A sample's report definition and CSV data, by name.
pub fn by_name(name: &str) -> Option<(Report, &'static str)> {
    match name {
        "rentals" => Some((rentals(), RENTALS_CSV)),
        _ => None,
    }
}

pub const NAMES: &[&str] = &["rentals"];

const RENTALS_CSV: &str = "\
branch,item,days,amount
Lyon,Scaffold tower,12,540.00
Lyon,Mini excavator,3,690.50
Grenoble,Generator 6 kVA,7,315.00
Lyon,Concrete mixer,5,125.00
Grenoble,Pressure washer,2,88.40
Annecy,Scissor lift,4,720.00
Annecy,Scaffold tower,9,405.00
";

fn text(x: f64, w: f64, content: Content, style: TextStyle) -> Element {
    Element {
        x_mm: x,
        y_mm: 0.5,
        width_mm: w,
        height_mm: 5.0,
        style,
        content,
        format: None,
        visible_if: None,
        background_formula: None,
        color_formula: None,
    }
}

/// Amounts: two decimals, grouped thousands.
fn money(el: Element) -> Element {
    Element { format: Some(Format::Number { decimals: 2, grouping: true }), ..el }
}

fn field(name: &str) -> Content {
    Content::Field { field: name.into() }
}

fn rentals() -> Report {
    let plain = TextStyle::default();
    let bold = TextStyle { bold: true, ..plain };
    let right = TextStyle { align: Align::Right, ..plain };
    let bold_right = TextStyle { bold: true, align: Align::Right, ..plain };
    let sum = |f: &str| Content::Aggregate { function: AggregateFn::Sum, field: f.into() };

    let mut r = Report::new("Rentals by branch");
    r.parameters.push(Parameter::text("period", "Period", Some("October 2026".into())));
    r.sort.push(SortKey { field: "amount".into(), descending: true });
    r.report_header = Band {
        height_mm: 14.0,
        elements: vec![
            text(0.0, 120.0, Content::Text { text: "Rentals by branch".into() }, TextStyle { size_pt: 16.0, bold: true, ..plain }),
            Element { y_mm: 8.0, ..text(0.0, 120.0, Content::Parameter { name: "period".into() }, plain) },
        ],
        page_break_before_if: None,
        visible_if: None,
    };
    r.page_header = Band {
        height_mm: 7.0,
        elements: vec![
            text(0.0, 80.0, Content::Text { text: "Item".into() }, bold),
            text(90.0, 30.0, Content::Text { text: "Days".into() }, bold_right),
            text(130.0, 50.0, Content::Text { text: "Amount".into() }, bold_right),
        ],
        page_break_before_if: None,
        visible_if: None,
    };
    r.groups.push(Group {
        field: "branch".into(),
        keep_together: false,
        repeat_header: false,
        page_break_before: false,
        header: Band { height_mm: 7.0, elements: vec![text(0.0, 120.0, field("branch"), bold)], page_break_before_if: None, visible_if: None },
        footer: Band {
            height_mm: 8.0,
            elements: vec![
                text(0.0, 80.0, Content::Text { text: "Branch total".into() }, bold),
                text(90.0, 30.0, sum("days"), bold_right),
                money(text(130.0, 50.0, sum("amount"), bold_right)),
            ],
            page_break_before_if: None,
            visible_if: None,
        },
    });
    r.detail = Band {
        height_mm: 6.0,
        elements: vec![
            text(5.0, 75.0, field("item"), plain),
            text(90.0, 30.0, field("days"), right),
            money(text(130.0, 50.0, field("amount"), right)),
        ],
        page_break_before_if: None,
        visible_if: None,
    };
    r.report_footer = Band {
        height_mm: 9.0,
        elements: vec![text(0.0, 80.0, Content::Text { text: "Grand total".into() }, bold), money(text(130.0, 50.0, sum("amount"), bold_right))],
        page_break_before_if: None,
        visible_if: None,
    };
    r.page_footer =
        Band { height_mm: 6.0, elements: vec![text(130.0, 50.0, Content::PageNumber, right)], page_break_before_if: None, visible_if: None };
    r
}
