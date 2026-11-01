//! Golden rendering tests (R2.13): reference reports, built in code, whose laid-out pages are
//! compared with snapshots in `tests/golden/*.txt`. A snapshot records every page and every item's
//! position, size, style and text, so any change to pagination or geometry shows up as a diff.
//!
//! When a change is intended, regenerate the snapshots and review the diff before committing:
//! `REPORTCRAFT_BLESS=1 cargo test -p reportcraft-engine --test golden`.

use std::fmt::Write as _;
use std::path::PathBuf;

use reportcraft_core::{Column, FieldType, RowSet, Value};
use reportcraft_engine::sample;
use reportcraft_layout::{Document, Params, layout};
use reportcraft_model::{AggregateFn, Align, Band, Content, Element, Group, Report, SortKey, TextStyle};

/// The snapshot text of a document: one header line per page, one line per item.
fn snapshot(doc: &Document) -> String {
    let mut out = format!("document {:?} {:.2}x{:.2}mm, {} page(s)\n", doc.title, doc.width_mm, doc.height_mm, doc.pages.len());
    for (i, page) in doc.pages.iter().enumerate() {
        let _ = writeln!(out, "page {} ({} items)", i + 1, page.items.len());
        for it in &page.items {
            let align = match it.style.align {
                Align::Left => "L",
                Align::Center => "C",
                Align::Right => "R",
            };
            let _ = writeln!(
                out,
                "  {:>7.2} {:>7.2} {:>6.2}x{:<5.2} {:>4.1}pt{}{} {:?}",
                it.x_mm,
                it.y_mm,
                it.width_mm,
                it.height_mm,
                it.style.size_pt,
                if it.style.bold { "B" } else { " " },
                align,
                it.text
            );
        }
    }
    out
}

fn check(name: &str, report: &Report, rows: &RowSet, params: &Params) {
    let doc = layout(report, rows, params).unwrap();
    let actual = snapshot(&doc);
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("{name}.txt"));
    if std::env::var_os("REPORTCRAFT_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    let expected =
        std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing snapshot {}; run with REPORTCRAFT_BLESS=1 to create it", path.display()));
    if expected != actual {
        let first = expected.lines().zip(actual.lines()).position(|(a, b)| a != b).unwrap_or(0);
        panic!(
            "{name}: rendering differs from {} at line {}\n  expected: {}\n  actual:   {}\n(run with REPORTCRAFT_BLESS=1 if the change is intended, then review the diff)",
            path.display(),
            first + 1,
            expected.lines().nth(first).unwrap_or("<end>"),
            actual.lines().nth(first).unwrap_or("<end>"),
        );
    }
}

fn el(x: f64, w: f64, content: Content, style: TextStyle) -> Element {
    Element { x_mm: x, y_mm: 0.5, width_mm: w, height_mm: 5.0, style, content }
}

fn field(name: &str) -> Content {
    Content::Field { field: name.into() }
}

fn agg(function: AggregateFn, field: &str) -> Content {
    Content::Aggregate { function, field: field.into() }
}

/// Deterministic invented data: `n` rentals over regions, branches and items.
fn rentals(n: usize) -> RowSet {
    let regions = ["North", "South"];
    let branches = ["Annecy", "Grenoble", "Lyon", "Valence"];
    let items = ["Generator", "Mini excavator", "Scaffold tower", "Scissor lift", "Pressure washer"];
    let col = |name: &str, t| Column { name: name.into(), field_type: t };
    RowSet {
        columns: vec![
            col("region", FieldType::Text),
            col("branch", FieldType::Text),
            col("item", FieldType::Text),
            col("days", FieldType::Integer),
            col("amount", FieldType::Number),
        ],
        rows: (0..n)
            .map(|i| {
                let days = (i * 7 % 13 + 1) as i64;
                vec![
                    Value::Text(regions[i % 2].into()),
                    Value::Text(branches[i * 3 % 4].into()),
                    Value::Text(items[i % 5].into()),
                    Value::Int(days),
                    Value::Number(days as f64 * 37.5 + (i % 4) as f64 * 0.25),
                ]
            })
            .collect(),
    }
}

#[test]
fn sample_rentals() {
    let (report, csv) = sample::by_name("rentals").unwrap();
    let rows = reportcraft_data::DataSource::read(&reportcraft_data::CsvSource::new(csv), None).unwrap();
    check("sample_rentals", &report, &rows, &Params::new());
}

#[test]
fn sample_rentals_with_parameter() {
    let (report, csv) = sample::by_name("rentals").unwrap();
    let rows = reportcraft_data::DataSource::read(&reportcraft_data::CsvSource::new(csv), None).unwrap();
    let params = Params::from([("period".to_owned(), "Q4 2026".to_owned())]);
    check("sample_rentals_with_parameter", &report, &rows, &params);
}

/// Two group levels over many rows on a small page: groups cross page breaks, page headers and
/// footers repeat, page numbers count the final total.
#[test]
fn nested_groups_across_pages() {
    let plain = TextStyle::default();
    let bold = TextStyle { bold: true, ..plain };
    let right = TextStyle { align: Align::Right, ..plain };
    let mut r = Report::new("Nested groups");
    r.page.width_mm = 148.0;
    r.page.height_mm = 105.0;
    r.page.margin_mm = 8.0;
    r.sort.push(SortKey { field: "amount".into(), descending: true });
    r.report_header = Band { height_mm: 9.0, elements: vec![el(0.0, 100.0, Content::Text { text: "Nested groups".into() }, bold)] };
    r.page_header = Band { height_mm: 6.0, elements: vec![el(0.0, 60.0, Content::Text { text: "Item".into() }, bold)] };
    r.groups = vec![
        Group {
            field: "region".into(),
            header: Band { height_mm: 7.0, elements: vec![el(0.0, 80.0, field("region"), bold)] },
            footer: Band { height_mm: 6.0, elements: vec![el(90.0, 40.0, agg(AggregateFn::Sum, "amount"), right)] },
        },
        Group {
            field: "branch".into(),
            header: Band { height_mm: 6.0, elements: vec![el(4.0, 80.0, field("branch"), bold)] },
            footer: Band {
                height_mm: 6.0,
                elements: vec![el(4.0, 40.0, agg(AggregateFn::Count, "item"), plain), el(90.0, 40.0, agg(AggregateFn::Avg, "amount"), right)],
            },
        },
    ];
    r.detail = Band { height_mm: 5.0, elements: vec![el(8.0, 60.0, field("item"), plain), el(90.0, 40.0, field("amount"), right)] };
    r.page_footer = Band { height_mm: 5.0, elements: vec![el(90.0, 40.0, Content::PageNumber, right)] };
    r.report_footer = Band {
        height_mm: 7.0,
        elements: vec![
            el(0.0, 40.0, agg(AggregateFn::Min, "days"), plain),
            el(45.0, 40.0, agg(AggregateFn::Max, "days"), plain),
            el(90.0, 40.0, agg(AggregateFn::Sum, "amount"), TextStyle { bold: true, ..right }),
        ],
    };
    check("nested_groups_across_pages", &r, &rentals(60), &Params::new());
}

/// No rows at all: headers, footers and empty aggregates still render on one page.
#[test]
fn empty_data() {
    let (report, _) = sample::by_name("rentals").unwrap();
    let rows = RowSet { columns: rentals(0).columns, rows: Vec::new() };
    check("empty_data", &report, &rows, &Params::new());
}

/// A detail band exactly as tall as the room left: no blank page, no early break.
#[test]
fn bands_that_exactly_fill_the_page() {
    let mut r = Report::new("Exact fit");
    r.page.height_mm = 100.0;
    r.page.margin_mm = 10.0;
    r.page_footer = Band { height_mm: 10.0, elements: vec![el(0.0, 40.0, Content::PageNumber, TextStyle::default())] };
    // 100 - 2 * 10 margin - 10 footer = 70 mm of body: seven 10 mm rows per page.
    r.detail = Band { height_mm: 10.0, elements: vec![el(0.0, 60.0, field("item"), TextStyle::default())] };
    check("bands_that_exactly_fill_the_page", &r, &rentals(14), &Params::new());
}
