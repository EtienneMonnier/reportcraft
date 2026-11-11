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
use reportcraft_layout::{Document, Params, Shape, layout};
use reportcraft_model::{AggregateFn, Align, Band, Content, Element, Format, Group, Report, SortKey, TextStyle};

/// The snapshot text of a document: one header line per page, one line per item.
fn snapshot(doc: &Document) -> String {
    let mut out = format!("document {:?} {:.2}x{:.2}mm, {} page(s)\n", doc.title, doc.width_mm, doc.height_mm, doc.pages.len());
    for (i, page) in doc.pages.iter().enumerate() {
        let _ = writeln!(out, "page {} ({} items)", i + 1, page.items.len());
        for it in &page.items {
            if let Some(shape) = &it.shape {
                let what = match shape {
                    Shape::Line { thickness_mm, color } => format!("line {thickness_mm:.2}mm {color}"),
                    Shape::Rect { border_mm, border_color, fill } => {
                        format!("rect {border_mm:.2}mm {border_color} fill {}", fill.as_deref().unwrap_or("none"))
                    }
                    // Linear: every module. Matrix: its size and dark-module count (the encoders
                    // have their own tests, checked against a decoder).
                    Shape::Barcode { columns, rows, quiet, modules } if *rows == 1 => format!("barcode {columns} modules quiet {quiet} {modules}"),
                    Shape::Barcode { columns, rows, quiet, modules } => {
                        format!("barcode {columns}x{rows} quiet {quiet}, {} dark", modules.bytes().filter(|&b| b == b'1').count())
                    }
                };
                let _ = writeln!(out, "  {:>7.2} {:>7.2} {:>6.2}x{:<5.2} {what}", it.x_mm, it.y_mm, it.width_mm, it.height_mm);
                continue;
            }
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
            // Colours only when set, so reports without them keep their snapshots.
            if it.style.color.is_some() || it.style.background.is_some() {
                let c = |c: Option<reportcraft_model::Color>| c.map_or_else(|| "-".to_owned(), |c| c.to_string());
                let _ = writeln!(out, "          colour {} on {}", c(it.style.color), c(it.style.background));
            }
        }
    }
    out
}

fn check(name: &str, report: &Report, rows: &RowSet, params: &Params) {
    check_doc(name, &layout(report, rows, params).unwrap());
}

fn check_doc(name: &str, doc: &Document) {
    let actual = snapshot(doc);
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
    r.report_header = Band {
        height_mm: 9.0,
        elements: vec![el(0.0, 100.0, Content::Text { text: "Nested groups".into() }, bold)],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    r.page_header = Band {
        height_mm: 6.0,
        elements: vec![el(0.0, 60.0, Content::Text { text: "Item".into() }, bold)],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    r.groups = vec![
        Group {
            field: "region".into(),
            keep_together: false,
            repeat_header: false,
            page_break_before: false,
            header: Band {
                height_mm: 7.0,
                elements: vec![el(0.0, 80.0, field("region"), bold)],
                page_break_before_if: None,
                visible_if: None,
                keep_with_next: false,
            },
            footer: Band {
                height_mm: 6.0,
                elements: vec![el(90.0, 40.0, agg(AggregateFn::Sum, "amount"), right)],
                page_break_before_if: None,
                visible_if: None,
                keep_with_next: false,
            },
        },
        Group {
            field: "branch".into(),
            keep_together: false,
            repeat_header: false,
            page_break_before: false,
            header: Band {
                height_mm: 6.0,
                elements: vec![el(4.0, 80.0, field("branch"), bold)],
                page_break_before_if: None,
                visible_if: None,
                keep_with_next: false,
            },
            footer: Band {
                height_mm: 6.0,
                elements: vec![el(4.0, 40.0, agg(AggregateFn::Count, "item"), plain), el(90.0, 40.0, agg(AggregateFn::Avg, "amount"), right)],
                page_break_before_if: None,
                visible_if: None,
                keep_with_next: false,
            },
        },
    ];
    r.detail = Band {
        height_mm: 5.0,
        elements: vec![el(8.0, 60.0, field("item"), plain), el(90.0, 40.0, field("amount"), right)],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    r.page_footer = Band {
        height_mm: 5.0,
        elements: vec![el(90.0, 40.0, Content::PageNumber, right)],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    r.report_footer = Band {
        height_mm: 7.0,
        elements: vec![
            el(0.0, 40.0, agg(AggregateFn::Min, "days"), plain),
            el(45.0, 40.0, agg(AggregateFn::Max, "days"), plain),
            el(90.0, 40.0, agg(AggregateFn::Sum, "amount"), TextStyle { bold: true, ..right }),
        ],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
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
    r.page_footer = Band {
        height_mm: 10.0,
        elements: vec![el(0.0, 40.0, Content::PageNumber, TextStyle::default())],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    // 100 - 2 * 10 margin - 10 footer = 70 mm of body: seven 10 mm rows per page.
    r.detail = Band {
        height_mm: 10.0,
        elements: vec![el(0.0, 60.0, field("item"), TextStyle::default())],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    check("bands_that_exactly_fill_the_page", &r, &rentals(14), &Params::new());
}

/// Locale-aware formats: French separators, euros after the amount, percentages, dates in the
/// locale's pattern and in an explicit one, and values a format doesn't fit.
#[test]
fn formats_in_french() {
    let plain = TextStyle::default();
    let right = TextStyle { align: Align::Right, ..plain };
    let fmt = |el: Element, f: Format| Element { format: Some(f), ..el };
    let eur = Format::Currency { symbol: "€".into(), decimals: 2, symbol_after: true };
    let mut r = Report::new("Formats");
    r.locale = "fr".into();
    r.detail = Band {
        height_mm: 6.0,
        elements: vec![
            fmt(el(0.0, 30.0, field("started"), plain), Format::Date { pattern: None }),
            fmt(el(32.0, 40.0, field("started"), plain), Format::Date { pattern: Some("%d/%m/%y %H:%M".into()) }),
            fmt(el(75.0, 35.0, field("amount"), right), eur.clone()),
            fmt(el(112.0, 25.0, field("ratio"), right), Format::Percent { decimals: 1 }),
            fmt(el(140.0, 30.0, field("note"), plain), Format::Number { decimals: 2, grouping: true }),
        ],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    r.report_footer = Band {
        height_mm: 6.0,
        elements: vec![
            fmt(el(75.0, 35.0, agg(AggregateFn::Sum, "amount"), right), eur.clone()),
            fmt(el(112.0, 25.0, agg(AggregateFn::Count, "amount"), right), eur),
            fmt(el(0.0, 70.0, Content::Parameter { name: "period".into() }, plain), Format::Date { pattern: None }),
        ],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    let col = |name: &str, t| Column { name: name.into(), field_type: t };
    let rows = RowSet {
        columns: vec![
            col("started", FieldType::Text),
            col("amount", FieldType::Number),
            col("ratio", FieldType::Number),
            col("note", FieldType::Text),
        ],
        rows: vec![
            vec![Value::Text("2026-10-01".into()), Value::Number(1234567.891), Value::Number(0.125), Value::Text("n/a".into())],
            vec![Value::Text("2026-10-03 14:05:00+02".into()), Value::Number(-0.004), Value::Number(1.0), Value::Text("42".into())],
            vec![Value::Null, Value::Null, Value::Null, Value::Null],
        ],
    };
    let params = Params::from([("period".to_owned(), "2026-10-01..2026-10-31".to_owned())]);
    check("formats_in_french", &r, &rows, &params);
}

/// Group pagination: each region starts a page and repeats its header on the pages it continues
/// on; each branch is kept on one page when it fits.
#[test]
fn group_pagination() {
    let plain = TextStyle::default();
    let bold = TextStyle { bold: true, ..plain };
    let mut r = Report::new("Group pagination");
    r.page.width_mm = 148.0;
    r.page.height_mm = 105.0;
    r.page.margin_mm = 8.0;
    r.page_header = Band {
        height_mm: 6.0,
        elements: vec![el(0.0, 60.0, Content::Text { text: "Item".into() }, bold)],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    r.groups = vec![
        Group {
            field: "region".into(),
            keep_together: false,
            repeat_header: true,
            page_break_before: true,
            header: Band {
                height_mm: 7.0,
                elements: vec![el(0.0, 80.0, field("region"), bold)],
                page_break_before_if: None,
                visible_if: None,
                keep_with_next: false,
            },
            footer: Band {
                height_mm: 6.0,
                elements: vec![el(0.0, 80.0, Content::Text { text: "end of region".into() }, plain)],
                page_break_before_if: None,
                visible_if: None,
                keep_with_next: false,
            },
        },
        Group {
            field: "branch".into(),
            keep_together: true,
            repeat_header: false,
            page_break_before: false,
            header: Band {
                height_mm: 6.0,
                elements: vec![el(4.0, 80.0, field("branch"), bold)],
                page_break_before_if: None,
                visible_if: None,
                keep_with_next: false,
            },
            footer: Band {
                height_mm: 6.0,
                elements: vec![el(4.0, 40.0, agg(AggregateFn::Count, "item"), plain)],
                page_break_before_if: None,
                visible_if: None,
                keep_with_next: false,
            },
        },
    ];
    r.detail = Band {
        height_mm: 5.0,
        elements: vec![el(8.0, 60.0, field("item"), plain)],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    r.page_footer = Band {
        height_mm: 5.0,
        elements: vec![el(90.0, 40.0, Content::PageNumber, plain)],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    check("group_pagination", &r, &rentals(24), &Params::new());
}

/// A5 landscape, three columns: group and detail bands flow down a column then the next, the
/// repeated group header starts each column, and the report footer sits below the columns.
#[test]
fn landscape_columns() {
    use reportcraft_model::{Orientation, PageSetup, Paper};
    let plain = TextStyle::default();
    let bold = TextStyle { bold: true, ..plain };
    let mut r = Report::new("Columns");
    r.page = PageSetup { margin_mm: 10.0, columns: 3, column_gap_mm: 4.0, ..PageSetup::paper(Paper::A5, Orientation::Landscape) };
    r.report_header = Band {
        height_mm: 8.0,
        elements: vec![el(0.0, 190.0, Content::Text { text: "Items by region".into() }, bold)],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    r.page_header = Band {
        height_mm: 6.0,
        elements: vec![el(0.0, 190.0, Content::Text { text: "Rentals".into() }, plain)],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    r.groups = vec![Group {
        field: "region".into(),
        keep_together: false,
        repeat_header: true,
        page_break_before: false,
        header: Band {
            height_mm: 6.0,
            elements: vec![el(0.0, 56.0, field("region"), bold)],
            page_break_before_if: None,
            visible_if: None,
            keep_with_next: false,
        },
        footer: Band {
            height_mm: 6.0,
            elements: vec![el(0.0, 56.0, agg(AggregateFn::Count, "item"), plain)],
            page_break_before_if: None,
            visible_if: None,
            keep_with_next: false,
        },
    }];
    r.detail = Band {
        height_mm: 5.0,
        elements: vec![el(2.0, 40.0, field("item"), plain), el(42.0, 14.0, field("days"), plain)],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    r.page_footer = Band {
        height_mm: 5.0,
        elements: vec![el(150.0, 40.0, Content::PageNumber, plain)],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    r.report_footer = Band {
        height_mm: 6.0,
        elements: vec![el(0.0, 190.0, agg(AggregateFn::Sum, "days"), bold)],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    check("landscape_columns", &r, &rentals(70), &Params::new());
}

/// Lines and rectangles: a rule under the page header, a shaded box behind each group header, a
/// framed total, a vertical rule between columns of figures.
#[test]
fn lines_and_boxes() {
    let plain = TextStyle::default();
    let bold = TextStyle { bold: true, ..plain };
    let shape = |x: f64, y: f64, w: f64, h: f64, content: Content| Element {
        x_mm: x,
        y_mm: y,
        width_mm: w,
        height_mm: h,
        style: plain,
        content,
        format: None,
        visible_if: None,
        background_formula: None,
        color_formula: None,
    };
    let line = |color: &str| Content::Line { thickness_mm: 0.3, color: color.into() };
    let mut r = Report::new("Shapes");
    r.page_header = Band {
        height_mm: 8.0,
        elements: vec![el(0.0, 60.0, Content::Text { text: "Item".into() }, bold), shape(0.0, 7.0, 180.0, 0.0, line("#333"))],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    r.groups = vec![Group {
        field: "branch".into(),
        keep_together: false,
        repeat_header: false,
        page_break_before: false,
        header: Band {
            height_mm: 7.0,
            elements: vec![
                shape(0.0, 0.5, 180.0, 6.0, Content::Rect { border_mm: 0.0, border_color: "#000".into(), fill: Some("#eeeeee".into()) }),
                el(2.0, 80.0, field("branch"), bold),
            ],
            page_break_before_if: None,
            visible_if: None,
            keep_with_next: false,
        },
        footer: Band {
            height_mm: 8.0,
            elements: vec![
                shape(120.0, 0.5, 60.0, 6.5, Content::Rect { border_mm: 0.4, border_color: "#1f4e79".into(), fill: None }),
                el(122.0, 56.0, agg(AggregateFn::Sum, "amount"), TextStyle { align: Align::Right, ..bold }),
            ],
            page_break_before_if: None,
            visible_if: None,
            keep_with_next: false,
        },
    }];
    r.detail = Band {
        height_mm: 5.0,
        elements: vec![
            el(4.0, 80.0, field("item"), plain),
            shape(118.0, 0.0, 0.0, 5.0, line("#999999")),
            el(122.0, 56.0, field("amount"), TextStyle { align: Align::Right, ..plain }),
        ],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    check("lines_and_boxes", &r, &rentals(6), &Params::new());
}

/// Labels: three across on A4, 38 mm high, filled left to right then row by row; a region header
/// closes a partly filled row and spans the first column.
#[test]
fn labels_across() {
    use reportcraft_model::{ColumnOrder, PageSetup, Paper};
    let plain = TextStyle::default();
    let bold = TextStyle { bold: true, ..plain };
    let mut r = Report::new("Labels");
    r.page = PageSetup {
        margin_mm: 10.0,
        columns: 3,
        column_gap_mm: 2.5,
        column_order: ColumnOrder::Across,
        ..PageSetup::paper(Paper::A4, reportcraft_model::Orientation::Portrait)
    };
    r.groups = vec![Group {
        field: "region".into(),
        keep_together: false,
        repeat_header: false,
        page_break_before: false,
        header: Band {
            height_mm: 8.0,
            elements: vec![el(0.0, 60.0, field("region"), bold)],
            page_break_before_if: None,
            visible_if: None,
            keep_with_next: false,
        },
        footer: Band::default(),
    }];
    r.detail = Band {
        height_mm: 38.0,
        elements: vec![
            el(3.0, 55.0, field("item"), bold),
            Element { y_mm: 8.0, ..el(3.0, 55.0, field("branch"), plain) },
            Element { y_mm: 15.0, ..el(3.0, 55.0, field("days"), plain) },
        ],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    // 13 North (4 full rows and 1 label) and 12 South labels.
    let mut rows = rentals(25);
    rows.rows.sort_by(|a, b| a[0].sort_cmp(&b[0]));
    check("labels_across", &r, &rows, &Params::new());
}

#[test]
fn barcodes() {
    use reportcraft_model::{EcLevel, Symbology};
    let plain = TextStyle::default();
    let bold = TextStyle { bold: true, ..plain };
    let mut r = Report::new("Asset labels");
    r.page = reportcraft_model::PageSetup { margin_mm: 10.0, columns: 2, column_gap_mm: 5.0, ..reportcraft_model::PageSetup::default() };
    let code = |symbology, field_name: &str, x: f64, y: f64, w: f64, h: f64| Element {
        x_mm: x,
        y_mm: y,
        width_mm: w,
        height_mm: h,
        style: plain,
        format: None,
        content: Content::Barcode { symbology, field: Some(field_name.into()), text: None, show_text: true, ec_level: EcLevel::M },
        visible_if: None,
        background_formula: None,
        color_formula: None,
    };
    r.report_header = Band {
        height_mm: 22.0,
        elements: vec![
            el(0.0, 120.0, Content::Text { text: "Asset labels".into() }, bold),
            Element {
                x_mm: 150.0,
                y_mm: 0.0,
                width_mm: 20.0,
                height_mm: 20.0,
                style: plain,
                format: None,
                content: Content::Barcode {
                    symbology: Symbology::Qr,
                    field: None,
                    text: Some("https://example.com/assets".into()),
                    show_text: false,
                    ec_level: EcLevel::Q,
                },
                visible_if: None,
                background_formula: None,
                color_formula: None,
            },
        ],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    r.detail = Band {
        height_mm: 50.0,
        elements: vec![
            el(0.0, 60.0, field("name"), bold),
            code(Symbology::Code128, "asset", 0.0, 7.0, 60.0, 15.0),
            code(Symbology::Ean13, "ean", 0.0, 24.0, 40.0, 20.0),
            code(Symbology::Qr, "url", 62.0, 7.0, 22.0, 22.0),
        ],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    let col = |name: &str| Column { name: name.into(), field_type: FieldType::Text };
    let text = |v: &str| if v.is_empty() { Value::Null } else { Value::Text(v.into()) };
    let rows = RowSet {
        columns: vec![col("name"), col("asset"), col("ean"), col("url")],
        rows: [
            ["Scaffold tower", "AS-00017", "400638133393", "https://example.com/a/17"],
            ["Mini excavator", "AS-00942", "4006381333931", "https://example.com/a/942"],
            // Not an EAN-13: written as text. No URL: no QR code.
            ["Generator 6 kVA", "AS-01003", "40063813339X", ""],
        ]
        .iter()
        .map(|r| r.iter().map(|v| text(v)).collect())
        .collect(),
    };
    check("barcodes", &r, &rows, &Params::new());
}

#[test]
fn subreports() {
    use reportcraft_model::{PageSetup, SubreportLink};
    let plain = TextStyle::default();
    let bold = TextStyle { bold: true, ..plain };
    let right = TextStyle { align: Align::Right, ..plain };
    let money = Some(Format::Number { decimals: 2, grouping: true });
    let at = |x: f64, w: f64, content: Content, style: TextStyle| Element { y_mm: 0.0, ..el(x, w, content, style) };

    // The rentals of one customer: titles, one line per rental, a total.
    let mut lines = Report::new("Rentals");
    lines.sort = vec![SortKey { field: "item".into(), descending: false }];
    lines.report_header = Band {
        height_mm: 5.0,
        elements: vec![
            at(0.0, 80.0, Content::Text { text: "Item".into() }, bold),
            at(80.0, 20.0, Content::Text { text: "Days".into() }, bold),
            at(100.0, 40.0, Content::Text { text: "Amount".into() }, TextStyle { align: Align::Right, ..bold }),
        ],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    lines.detail = Band {
        height_mm: 5.0,
        elements: vec![
            at(0.0, 80.0, field("item"), plain),
            at(80.0, 20.0, field("days"), plain),
            Element { format: money.clone(), ..at(100.0, 40.0, field("amount"), right) },
        ],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    lines.report_footer = Band {
        height_mm: 6.0,
        elements: vec![
            at(0.0, 80.0, Content::Text { text: "Total".into() }, bold),
            Element { format: money.clone(), ..at(100.0, 40.0, agg(AggregateFn::Sum, "amount"), TextStyle { align: Align::Right, ..bold }) },
        ],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };

    let mut r = Report::new("Statements");
    r.page = PageSetup { height_mm: 120.0, margin_mm: 10.0, ..PageSetup::default() };
    r.page_header = Band {
        height_mm: 7.0,
        elements: vec![el(0.0, 100.0, Content::Text { text: "Customer statements".into() }, bold)],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    r.page_footer = Band {
        height_mm: 5.0,
        elements: vec![el(150.0, 40.0, Content::PageNumber, right)],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    r.detail = Band {
        height_mm: 14.0,
        elements: vec![
            el(0.0, 120.0, field("name"), bold),
            Element {
                x_mm: 5.0,
                y_mm: 6.0,
                width_mm: 140.0,
                height_mm: 6.0,
                style: plain,
                format: None,
                content: Content::Subreport {
                    report: Box::new(lines),
                    dataset: "rentals".into(),
                    links: vec![SubreportLink { parent: "id".into(), child: "customer".into() }],
                    hide_when_empty: true,
                },
                visible_if: None,
                background_formula: None,
                color_formula: None,
            },
        ],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    r.report_footer = Band {
        height_mm: 6.0,
        elements: vec![el(0.0, 100.0, Content::Text { text: "End of statements".into() }, plain)],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };

    let col = |name: &str, t| Column { name: name.into(), field_type: t };
    let customers = RowSet {
        columns: vec![col("id", FieldType::Integer), col("name", FieldType::Text)],
        rows: [(1, "Alpine BTP"), (2, "Chantiers du Lac"), (3, "Rhône Démolition"), (4, "Savoie Toitures")]
            .iter()
            .map(|(id, name)| vec![Value::Int(*id), Value::Text((*name).into())])
            .collect(),
    };
    // Customer ids as text on this side: links match `1` and `"1"`. Customer 3 rented nothing.
    let items = ["Generator", "Mini excavator", "Scaffold tower", "Scissor lift", "Pressure washer"];
    let mut rentals = vec![];
    for (customer, n) in [(1, 3), (2, 2), (4, 22)] {
        for k in 0..n {
            let item = items[(k * 3 + customer) % items.len()];
            let days = (k * 7 + customer) % 12 + 1;
            rentals.push(vec![
                Value::Text(customer.to_string()),
                Value::Text(format!("{item} #{}", k + 1)),
                Value::Int(days as i64),
                Value::Number(days as f64 * 42.5),
            ]);
        }
    }
    let rentals = RowSet {
        columns: vec![
            col("customer", FieldType::Text),
            col("item", FieldType::Text),
            col("days", FieldType::Integer),
            col("amount", FieldType::Number),
        ],
        rows: rentals,
    };
    let mut datasets = reportcraft_layout::Datasets::new();
    datasets.insert("rentals".into(), std::sync::Arc::new(rentals));
    let doc = reportcraft_layout::render(&r, &customers, &datasets, &Params::new(), &reportcraft_core::Cancel::default()).unwrap();
    check_doc("subreports", &doc);
}

/// Formulas (R2.2): named formulas used as fields, a group on a computed field, and formula
/// elements summarising the group, in a French layout.
#[test]
fn formulas() {
    use reportcraft_model::FormulaField;
    let plain = TextStyle::default();
    let bold = TextStyle { bold: true, ..plain };
    let right = TextStyle { align: Align::Right, ..plain };
    let formula = |e: &str| Content::Formula { expression: e.into() };
    let money = |el: Element| Element { format: Some(Format::Currency { symbol: "€".into(), decimals: 2, symbol_after: true }), ..el };
    let mut r = Report::new("Rentals by duration");
    r.locale = "fr".into();
    r.formulas = vec![
        FormulaField { name: "daily".into(), expression: "ROUND([amount] / [days], 2)".into() },
        FormulaField { name: "length".into(), expression: "IFS([days] <= 3, \"1. Short\", [days] <= 8, \"2. Medium\", TRUE, \"3. Long\")".into() },
        FormulaField { name: "label".into(), expression: "UPPER(LEFT([branch], 3)) & \"-\" & [item]".into() },
    ];
    r.sort.push(SortKey { field: "daily".into(), descending: true });
    r.groups.push(Group {
        field: "length".into(),
        keep_together: false,
        repeat_header: true,
        page_break_before: false,
        header: Band {
            height_mm: 7.0,
            elements: vec![el(0.0, 120.0, formula("MID([length], 4, 20) & \" rentals (\" & COUNT([days]) & \")\""), bold)],
            page_break_before_if: None,
            visible_if: None,
            keep_with_next: false,
        },
        footer: Band {
            height_mm: 7.0,
            elements: vec![
                el(
                    0.0,
                    80.0,
                    formula("\"Average \" & ROUND(AVERAGE([days]), 1) & \" days, \" & COUNTIF([region], \"North\") & \" in the North\""),
                    plain,
                ),
                money(el(140.0, 40.0, formula("SUM([amount])"), right)),
            ],
            page_break_before_if: None,
            visible_if: None,
            keep_with_next: false,
        },
    });
    r.detail = Band {
        height_mm: 5.5,
        elements: vec![
            el(0.0, 70.0, field("label"), plain),
            el(70.0, 15.0, field("days"), right),
            money(el(85.0, 30.0, field("daily"), right)),
            Element { format: Some(Format::Percent { decimals: 1 }), ..el(115.0, 25.0, formula("[amount] / SUM([amount])"), right) },
            money(el(140.0, 40.0, field("amount"), right)),
        ],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    // Report footer: a share computed from conditional sums.
    r.report_footer = Band { height_mm: 7.0, elements: Vec::new(), page_break_before_if: None, visible_if: None, keep_with_next: false };
    r.report_footer.elements = vec![
        el(
            0.0,
            120.0,
            formula("\"Long rentals bring \" & ROUND(SUMIF([length], \"3*\", [amount]) / SUM([amount]) * 100, 1) & \" % of the total\""),
            plain,
        ),
        money(el(140.0, 40.0, formula("SUM([amount])"), right)),
    ];
    check("formulas", &r, &rentals(24), &Params::new());
}

/// Conditions (R2.10): rows, elements and footers shown on a formula, a page break on a value.
#[test]
fn conditions() {
    let plain = TextStyle::default();
    let bold = TextStyle { bold: true, ..plain };
    let right = TextStyle { align: Align::Right, ..plain };
    let cond = |c: &str| Some(c.to_owned());
    let mut r = Report::new("Long rentals");
    r.page_header = Band { height_mm: 7.0, elements: vec![el(0.0, 100.0, Content::Text { text: "Long rentals".into() }, bold)], ..Band::default() };
    r.groups.push(Group {
        field: "region".into(),
        keep_together: false,
        repeat_header: false,
        page_break_before: false,
        header: Band {
            height_mm: 7.0,
            elements: vec![el(0.0, 60.0, field("region"), bold)],
            page_break_before_if: cond("[region] = \"South\""),
            ..Band::default()
        },
        footer: Band {
            height_mm: 7.0,
            elements: vec![
                el(0.0, 60.0, Content::Formula { expression: "COUNT([days]) & \" rentals\"".into() }, plain),
                el(120.0, 40.0, agg(AggregateFn::Sum, "amount"), right),
            ],
            visible_if: cond("COUNT([days]) > 3"),
            ..Band::default()
        },
    });
    r.sort.push(SortKey { field: "days".into(), descending: true });
    r.detail = Band {
        height_mm: 5.5,
        elements: vec![
            el(0.0, 60.0, field("item"), plain),
            el(60.0, 20.0, field("days"), right),
            Element { visible_if: cond("[days] >= 10"), ..el(85.0, 30.0, Content::Text { text: "two weeks".into() }, bold) },
            el(120.0, 40.0, field("amount"), right),
        ],
        visible_if: cond("[days] > 2"),
        ..Band::default()
    };
    check("conditions", &r, &rentals(24), &Params::new());
}

/// Colours (R2.4): a coloured header on a background, and amounts below 100 in red by formula.
#[test]
fn colours() {
    use reportcraft_model::Color;
    let header = TextStyle { bold: true, color: Some(Color(0xff, 0xff, 0xff)), background: Some(Color(0x33, 0x55, 0x99)), ..TextStyle::default() };
    let right = TextStyle { align: Align::Right, ..TextStyle::default() };
    let mut r = Report::new("Colours");
    r.page_header = Band {
        height_mm: 7.0,
        elements: vec![el(0.0, 60.0, Content::Text { text: "Item".into() }, header), el(60.0, 40.0, Content::Text { text: "Amount".into() }, header)],
        ..Band::default()
    };
    r.detail = Band {
        height_mm: 5.5,
        elements: vec![
            el(0.0, 60.0, field("item"), TextStyle::default()),
            Element {
                color_formula: Some("IF([amount] < 100, \"#c00\", \"\")".into()),
                background_formula: Some("IF([days] >= 10, \"#ffeecc\", \"\")".into()),
                ..el(60.0, 40.0, field("amount"), right)
            },
        ],
        ..Band::default()
    };
    check("colours", &r, &rentals(8), &Params::new());
}
