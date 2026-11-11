//! PDF writer contracts. They need a font file: a craft-fonts checkout (`CRAFT_FONTS_DIR`, as in
//! CI), `REPORTCRAFT_FONT`, or a common system font. Without one they are skipped locally and
//! fail on CI (`CI` set), so the gate can't pass without running them.

use reportcraft_core::{Cancel, Error};
use reportcraft_layout::{Document, Item, Page, Shape};
use reportcraft_model::{Align, TextStyle};
use reportcraft_pdf::{Fonts, to_pdf};

fn fonts() -> Option<Fonts> {
    match Fonts::find() {
        Ok(f) => Some(f),
        Err(e) if std::env::var_os("CI").is_some() => panic!("no font on CI: {e}"),
        Err(e) => {
            eprintln!("skipped: {e}");
            None
        }
    }
}

fn text(x: f64, y: f64, text: &str, align: Align, bold: bool) -> Item {
    Item {
        x_mm: x,
        y_mm: y,
        width_mm: 60.0,
        height_mm: 5.0,
        style: TextStyle { size_pt: 10.0, bold, align },
        text: text.into(),
        shape: None,
        cell: None,
    }
}

fn shape(shape: Shape) -> Item {
    Item { x_mm: 10.0, y_mm: 50.0, width_mm: 40.0, height_mm: 10.0, style: TextStyle::default(), text: String::new(), shape: Some(shape), cell: None }
}

fn doc(pages: usize) -> Document {
    let page = Page {
        items: vec![
            text(10.0, 10.0, "Échafaudage roulant", Align::Left, true),
            text(10.0, 20.0, "14\u{202f}287,50\u{a0}€", Align::Right, false),
            text(10.0, 30.0, "line\nbreak", Align::Center, false),
            shape(Shape::Line { thickness_mm: 0.3, color: "#000".into() }),
            shape(Shape::Rect { border_mm: 0.2, border_color: "#333333".into(), fill: Some("#eeeeee".into()) }),
            shape(Shape::Rect { border_mm: 0.2, border_color: "not a colour".into(), fill: None }),
            shape(Shape::Barcode { columns: 3, rows: 1, quiet: 1, modules: "101".into() }),
            shape(Shape::Barcode { columns: 3, rows: 1, quiet: 1, modules: "10".into() }),
        ],
    };
    Document { title: "Test".into(), width_mm: 210.0, height_mm: 297.0, margin_mm: 15.0, pages: vec![page; pages] }
}

#[test]
fn writes_one_pdf_page_per_page_reproducibly() {
    let Some(fonts) = fonts() else { return };
    let pdf = to_pdf(&doc(3), &fonts, "ReportCraft test", &Cancel::default()).unwrap();
    assert!(pdf.bytes.starts_with(b"%PDF-"));
    let body = String::from_utf8_lossy(&pdf.bytes);
    assert!(body.contains("/Count 3"), "three pages");
    assert!(body.contains("/FontFile2") || body.contains("/FontFile3"), "the font is embedded");
    assert!(body.contains("/ToUnicode"), "text can be copied back out");
    // No clock, no random ids: the same document gives the same file.
    let again = to_pdf(&doc(3), &fonts, "ReportCraft test", &Cancel::default()).unwrap();
    assert_eq!(pdf.bytes, again.bytes);
    if fonts.bold.is_some() {
        assert!(pdf.warnings.is_empty(), "{:?}", pdf.warnings);
    }
}

#[test]
fn reports_characters_the_font_lacks() {
    let Some(fonts) = fonts() else { return };
    let mut d = doc(1);
    // A private-use code point no general-purpose font draws.
    d.pages[0].items.push(text(10.0, 40.0, "\u{f8ff}\u{e000}", Align::Left, false));
    let pdf = to_pdf(&d, &fonts, "t", &Cancel::default()).unwrap();
    assert!(pdf.warnings.iter().any(|w| w.contains("no glyph")), "{:?}", pdf.warnings);
}

#[test]
fn stops_when_cancelled() {
    let Some(fonts) = fonts() else { return };
    let cancel = Cancel::default();
    cancel.cancel();
    let big = doc(200);
    assert!(matches!(to_pdf(&big, &fonts, "t", &cancel), Err(Error::Cancelled(_))));
}

#[test]
fn refuses_a_file_that_is_not_a_font() {
    let fonts = Fonts::from_bytes(b"not a font".to_vec(), None);
    let err = to_pdf(&doc(1), &fonts, "t", &Cancel::default()).unwrap_err();
    assert!(matches!(&err, Error::Unsupported(m) if m.contains("regular font")), "{err}");
}

#[test]
fn a_page_size_of_zero_is_an_error_not_a_crash() {
    let Some(fonts) = fonts() else { return };
    let mut d = doc(1);
    d.width_mm = 0.0;
    assert!(matches!(to_pdf(&d, &fonts, "t", &Cancel::default()), Err(Error::Invalid(_))));
}
