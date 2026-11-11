//! The writers over a real layout: what the files hold, read back from their XML.

use std::io::Read;

use reportcraft_core::{Cancel, Column, FieldType, RowSet, Value};
use reportcraft_layout::{Datasets, Params, layout, render_continuous};
use reportcraft_model::{Align, Band, Content, Element, Format, Report, TextStyle};
use reportcraft_office::{to_docx, to_xlsx};

fn el(x: f64, w: f64, content: Content, format: Option<Format>, align: Align) -> Element {
    Element {
        x_mm: x,
        y_mm: 0.5,
        width_mm: w,
        height_mm: 5.0,
        style: TextStyle { align, ..TextStyle::default() },
        content,
        format,
        visible_if: None,
        background_formula: None,
        color_formula: None,
    }
}

fn report() -> (Report, RowSet) {
    let mut r = Report::new("Rentals: Q4 / 2026");
    r.page.height_mm = 80.0;
    r.page_header = Band {
        height_mm: 6.0,
        elements: vec![
            el(0.0, 60.0, Content::Text { text: "Item".into() }, None, Align::Left),
            el(60.0, 30.0, Content::Text { text: "Since".into() }, None, Align::Left),
            el(90.0, 40.0, Content::Text { text: "Amount".into() }, None, Align::Right),
        ],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    r.page_footer = Band {
        height_mm: 6.0,
        elements: vec![el(0.0, 30.0, Content::PageNumber, None, Align::Left)],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    let money = Format::Currency { symbol: "€".into(), decimals: 2, symbol_after: true };
    r.detail = Band {
        height_mm: 6.0,
        elements: vec![
            el(0.0, 60.0, Content::Field { field: "item".into() }, None, Align::Left),
            el(60.0, 30.0, Content::Field { field: "since".into() }, Some(Format::Date { pattern: Some("%d/%m/%Y".into()) }), Align::Left),
            Element {
                color_formula: Some("IF([amount] < 1240, \"#c00\", \"\")".into()),
                ..el(90.0, 40.0, Content::Field { field: "amount".into() }, Some(money), Align::Right)
            },
            el(140.0, 20.0, Content::Line { thickness_mm: 0.2, color: "#000".into() }, None, Align::Left),
        ],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    let col = |n: &str, t| Column { name: n.into(), field_type: t };
    let rows = RowSet {
        columns: vec![col("item", FieldType::Text), col("since", FieldType::Text), col("amount", FieldType::Number)],
        rows: (0..12)
            .map(|i| {
                vec![
                    Value::Text(format!("Scaffold & tower <{i}>")),
                    Value::Text(format!("2026-10-{:02}", i + 1)),
                    Value::Number(1234.5 + f64::from(i)),
                ]
            })
            .collect(),
    };
    (r, rows)
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = String::new();
    zip.by_name(name).unwrap().read_to_string(&mut out).unwrap();
    out
}

#[test]
fn xlsx_keeps_numbers_and_dates_typed() {
    let (r, rows) = report();
    let doc = render_continuous(&r, &rows, &Datasets::new(), &Params::new(), &Cancel::default()).unwrap();
    let out = to_xlsx(&doc, &Cancel::default()).unwrap();
    assert_eq!(out.warnings, ["12 line(s), box(es) or barcode(s) are not written to XLSX yet"]);
    let sheet = part(&out.bytes, "xl/worksheets/sheet1.xml");
    let styles = part(&out.bytes, "xl/styles.xml");
    let strings = part(&out.bytes, "xl/sharedStrings.xml");
    let workbook = part(&out.bytes, "xl/workbook.xml");
    assert!(workbook.contains("name=\"Rentals Q4  2026\""), "{workbook}");
    // Header once, then 12 rows; no page footer.
    assert_eq!(sheet.matches("<row ").count(), 13, "{sheet}");
    assert!(sheet.contains("<c r=\"C2\" s=\"") && sheet.contains("<v>1234.5</v>"), "amounts are numbers: {sheet}");
    // 2026-10-01 is Excel day 46296.
    assert!(sheet.contains("<v>46296</v>"), "dates are dates: {sheet}");
    assert!(styles.contains("formatCode=\"#,##0.00 &quot;€&quot;\""), "{styles}");
    assert!(styles.contains("formatCode=\"dd/mm/yyyy\""), "{styles}");
    assert!(styles.contains("rgb=\"FFCC0000\""), "red amounts: {styles}");
    assert!(strings.contains("Scaffold &amp; tower &lt;0&gt;") && !strings.contains("1 / 1"), "{strings}");
    assert_eq!(to_xlsx(&doc, &Cancel::default()).unwrap().bytes, out.bytes, "the same report gives the same file");
}

#[test]
fn docx_keeps_the_pages_and_positions() {
    let (r, rows) = report();
    let doc = layout(&r, &rows, &Params::new()).unwrap();
    assert_eq!(doc.pages.len(), 2);
    let out = to_docx(&doc, &Cancel::default()).unwrap();
    let xml = part(&out.bytes, "word/document.xml");
    assert_eq!(xml.matches("<w:pageBreakBefore").count(), 1, "one break between two pages");
    assert!(xml.contains("Scaffold &amp; tower &lt;0&gt;") && xml.contains("1 / 2") && xml.contains("2 / 2"));
    // Amounts sit on a right tab at the right edge of their box (15 + 90 + 40 mm, less the margin).
    assert!(xml.contains("<w:tab w:val=\"right\" w:leader=\"none\" w:pos=\"7370\""), "{xml}");
    assert!(xml.contains("<w:spacing w:before=\"0\" w:after=\"0\" w:line=\""), "{xml}");
    assert!(xml.contains("w:lineRule=\"exact\""));
    assert!(xml.contains("<w:color w:val=\"cc0000\""), "red amounts: {xml}");
    let section = xml.rsplit("<w:sectPr").next().unwrap();
    assert!(section.contains("w:w=\"11906\"") && section.contains("w:h=\"4535\""), "page size: {section}");
    assert_eq!(out.warnings.len(), 1);
    assert_eq!(to_docx(&doc, &Cancel::default()).unwrap().bytes, out.bytes, "the same report gives the same file");
}

#[test]
fn a_cancelled_export_stops_and_empty_reports_still_write() {
    let (r, rows) = report();
    let doc = layout(&r, &rows, &Params::new()).unwrap();
    let cancel = Cancel::default();
    cancel.cancel();
    assert!(to_docx(&doc, &cancel).is_err());
    assert!(to_xlsx(&doc, &cancel).is_err());
    let empty = layout(&Report::new(""), &RowSet::default(), &Params::new()).unwrap();
    assert!(!to_docx(&empty, &Cancel::default()).unwrap().bytes.is_empty());
    let sheet = to_xlsx(&empty, &Cancel::default()).unwrap();
    assert!(part(&sheet.bytes, "xl/workbook.xml").contains("name=\"Report\""));
}
