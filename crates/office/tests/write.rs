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
    assert_eq!(out.warnings, ["12 line(s), box(es), barcode(s) or picture(s) are not written to XLSX (not supported yet, or a damaged picture)"]);
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

#[test]
fn docx_draws_backgrounds_lines_boxes_and_barcodes_behind_the_text() {
    use reportcraft_model::{Color, EcLevel, Symbology};
    let shape = |x: f64, w: f64, h: f64, content: Content| Element {
        x_mm: x,
        y_mm: 2.0,
        width_mm: w,
        height_mm: h,
        style: TextStyle::default(),
        content,
        format: None,
        visible_if: None,
        color_formula: None,
        background_formula: None,
    };
    let mut r = Report::new("Shapes");
    r.report_header = Band {
        height_mm: 40.0,
        elements: vec![
            shape(0.0, 100.0, 0.0, Content::Line { thickness_mm: 0.5, color: "#335599".into() }),
            shape(0.0, 40.0, 20.0, Content::Rect { border_mm: 0.4, border_color: "#c00".into(), fill: Some("#ffeecc".into()) }),
            shape(
                50.0,
                30.0,
                30.0,
                Content::Barcode { symbology: Symbology::Qr, field: None, text: Some("x".into()), show_text: false, ec_level: EcLevel::M },
            ),
            shape(90.0, 20.0, 10.0, Content::Line { thickness_mm: 0.3, color: "#000".into() }),
            Element {
                style: TextStyle { background: Some(Color(0, 0, 0xff)), ..TextStyle::default() },
                ..shape(120.0, 30.0, 6.0, Content::Text { text: "on blue".into() })
            },
        ],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    let doc = layout(&r, &RowSet::default(), &Params::new()).unwrap();
    let out = to_docx(&doc, &Cancel::default()).unwrap();
    // The diagonal line is the only shape left out.
    assert_eq!(out.warnings, ["1 line(s), box(es), barcode(s) or picture(s) are not written to DOCX (not supported yet, or a damaged picture)"]);
    let xml = part(&out.bytes, "word/document.xml");
    // A line, a fill and four border edges, a barcode, a background: 8 pictures.
    assert_eq!(xml.matches("<wp:anchor ").count(), 8, "{xml}");
    assert_eq!(xml.matches(r#"behindDoc="1""#).count(), 8);
    assert_eq!(xml.matches("<wp:wrapNone />").count(), 8);
    assert!(!xml.contains("wrapSquare"));
    let mut ids: Vec<&str> = xml.match_indices(r#"<wp:docPr id=""#).map(|(i, _)| xml.get(i + 14..i + 16).unwrap_or("")).collect();
    let n = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), n, "every drawing has its own id");
    assert!(xml.contains("on blue"));
    assert_eq!(to_docx(&doc, &Cancel::default()).unwrap().bytes, out.bytes, "still the same bytes for the same report");
}

#[test]
fn docx_draws_chart_wedges_as_pictures() {
    let (mut r, rows) = report();
    r.detail.elements.retain(|e| !matches!(e.content, Content::Line { .. }));
    r.report_footer = Band {
        height_mm: 50.0,
        elements: vec![Element {
            content: Content::Chart {
                chart: reportcraft_model::ChartKind::Pie,
                category: "since".into(),
                value: "amount".into(),
                function: reportcraft_model::AggregateFn::Sum,
                title: None,
            },
            ..el(0.0, 120.0, Content::Text { text: String::new() }, None, Align::Left)
        }],
        page_break_before_if: None,
        visible_if: None,
        keep_with_next: false,
    };
    if let Some(e) = r.report_footer.elements.first_mut() {
        e.height_mm = 45.0;
    }
    r.page.height_mm = 297.0;
    let doc = layout(&r, &rows, &Params::new()).unwrap();
    let out = to_docx(&doc, &Cancel::default()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let xml = part(&out.bytes, "word/document.xml");
    // 12 wedges and their 12 legend squares that fit (as many as rows of the legend fit).
    assert!(xml.matches("<wp:anchor ").count() >= 12, "{}", xml.matches("<wp:anchor ").count());
}

#[test]
fn docx_draws_pictures_with_jpeg_converted_to_png() {
    use reportcraft_core::image::{ImageData, Picture};
    use reportcraft_layout::pixels::{Rgba, decode, encode_png};
    use reportcraft_model::ImageFit;
    let png = encode_png(&Rgba { width: 3, height: 1, pixels: [0, 128, 255, 255].repeat(3) }).unwrap();
    let jpeg = include_bytes!("../../layout/tests/pictures/red-blue-4x2.jpg");
    let webp = include_bytes!("../../layout/tests/pictures/red-blue-4x2.webp");
    let image = |x: f64, bytes: &[u8]| Element {
        x_mm: x,
        y_mm: 0.0,
        width_mm: 30.0,
        height_mm: 30.0,
        content: Content::Image { data: Some(ImageData(bytes.into())), field: None, fit: ImageFit::Contain },
        ..el(0.0, 1.0, Content::Text { text: String::new() }, None, Align::Left)
    };
    let mut r = Report::new("Pictures");
    r.report_header = Band { height_mm: 40.0, elements: vec![image(0.0, &png), image(40.0, jpeg), image(80.0, webp)], ..Band::default() };
    let doc = layout(&r, &RowSet::default(), &Params::new()).unwrap();
    let out = to_docx(&doc, &Cancel::default()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let xml = part(&out.bytes, "word/document.xml");
    assert_eq!(xml.matches("<wp:anchor ").count(), 3, "{xml}");
    // 3 × 1 pixels kept in proportion in a 30 mm square: 30 × 10 mm, as EMUs.
    assert!(xml.contains(r#"<wp:extent cx="1080000" cy="360000" />"#), "{xml}");
    // Every media file is PNG; the JPEG's and the WebP's colours survive the conversion.
    let media = media_parts(&out.bytes);
    assert_eq!(media.len(), 3);
    assert!(media.contains(&png));
    for converted in media.iter().filter(|m| **m != png) {
        let picture = Picture::read(converted).unwrap();
        assert_eq!(picture.format, reportcraft_core::image::ImageFormat::Png);
        let pixels = decode(&picture).unwrap();
        assert_eq!((pixels.width, pixels.height), (4, 2));
        assert!(pixels.pixels[0] > 200 && pixels.pixels[2] < 60, "red on the left");
    }
}

/// The files under `word/media/`.
fn media_parts(docx: &[u8]) -> Vec<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(docx)).unwrap();
    let names: Vec<String> =
        zip.file_names().filter_map(Result::ok).filter(|n| n.starts_with("word/media/") && !n.ends_with('/')).map(|n| n.into_owned()).collect();
    names
        .iter()
        .map(|n| {
            let mut bytes = Vec::new();
            zip.by_name(n).unwrap().read_to_end(&mut bytes).unwrap();
            bytes
        })
        .collect()
}
