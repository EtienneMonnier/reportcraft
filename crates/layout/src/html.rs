//! HTML writer: one absolutely positioned box per item, one sheet per page, printable as is.

use std::fmt::Write as _;

use reportcraft_model::Align;

use crate::{Document, Shape, color_is_valid};

/// The document as a standalone HTML page.
pub fn to_html(doc: &Document) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>{}</title><style>\
         body{{margin:0;background:#e9e9e9;font-family:sans-serif}}\
         .page{{position:relative;background:#fff;margin:8mm auto;width:{w}mm;height:{h}mm;overflow:hidden}}\
         .item{{position:absolute;white-space:nowrap;overflow:hidden;line-height:1.2}}\
         @page{{size:{w}mm {h}mm;margin:0}}\
         @media print{{body{{background:none}}.page{{margin:0;break-after:page}}}}\
         </style></head><body>\n",
        escape(&doc.title),
        w = doc.width_mm,
        h = doc.height_mm,
    );
    for page in &doc.pages {
        out.push_str("<div class=\"page\">\n");
        shapes(&mut out, page, doc.width_mm, doc.height_mm);
        for it in page.items.iter().filter(|i| i.shape.is_none()) {
            let align = match it.style.align {
                Align::Left => "left",
                Align::Center => "center",
                Align::Right => "right",
            };
            let _ = writeln!(
                out,
                "<div class=\"item\" style=\"left:{:.2}mm;top:{:.2}mm;width:{:.2}mm;height:{:.2}mm;font-size:{:.1}pt;text-align:{align}{}\">{}</div>",
                it.x_mm,
                it.y_mm,
                it.width_mm,
                it.height_mm,
                it.style.size_pt,
                if it.style.bold { ";font-weight:bold" } else { "" },
                escape(&it.text),
            );
        }
        out.push_str("</div>\n");
    }
    out.push_str("</body></html>\n");
    out
}

/// The page's lines and rectangles as one SVG layer under the text, in millimetres.
fn shapes(out: &mut String, page: &crate::Page, width: f64, height: f64) {
    let drawn: Vec<(&crate::Item, &Shape)> = page.items.iter().filter_map(|i| i.shape.as_ref().map(|s| (i, s))).collect();
    if drawn.is_empty() {
        return;
    }
    let _ = writeln!(
        out,
        "<svg class=\"shapes\" style=\"position:absolute;left:0;top:0;width:{width}mm;height:{height}mm\" viewBox=\"0 0 {width} {height}\" xmlns=\"http://www.w3.org/2000/svg\">"
    );
    for (it, shape) in drawn {
        match shape {
            Shape::Line { thickness_mm, color } if color_is_valid(color) => {
                let _ = writeln!(
                    out,
                    "<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"{color}\" stroke-width=\"{thickness_mm:.2}\"/>",
                    it.x_mm,
                    it.y_mm,
                    it.x_mm + it.width_mm,
                    it.y_mm + it.height_mm
                );
            }
            Shape::Rect { border_mm, border_color, fill } if color_is_valid(border_color) && fill.as_deref().is_none_or(color_is_valid) => {
                let fill = fill.as_deref().unwrap_or("none");
                let stroke = if *border_mm > 0.0 { border_color.as_str() } else { "none" };
                let _ = writeln!(
                    out,
                    "<rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"{border_mm:.2}\"/>",
                    it.x_mm, it.y_mm, it.width_mm, it.height_mm
                );
            }
            // A colour that isn't #rgb/#rrggbb never reaches the page.
            _ => {}
        }
    }
    out.push_str("</svg>\n");
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Item, Page};
    use reportcraft_model::TextStyle;

    #[test]
    fn escapes_text() {
        let doc = Document {
            title: "<t>".into(),
            width_mm: 10.0,
            height_mm: 10.0,
            pages: vec![Page {
                items: vec![
                    Item { x_mm: 0.0, y_mm: 0.0, width_mm: 1.0, height_mm: 1.0, style: TextStyle::default(), text: "a<b&\"".into(), shape: None },
                    Item {
                        x_mm: 0.0,
                        y_mm: 0.0,
                        width_mm: 5.0,
                        height_mm: 0.0,
                        style: TextStyle::default(),
                        text: String::new(),
                        shape: Some(Shape::Line { thickness_mm: 0.3, color: "red\"/><script>".into() }),
                    },
                    Item {
                        x_mm: 1.0,
                        y_mm: 2.0,
                        width_mm: 3.0,
                        height_mm: 4.0,
                        style: TextStyle::default(),
                        text: String::new(),
                        shape: Some(Shape::Rect { border_mm: 0.5, border_color: "#123".into(), fill: Some("#abcdef".into()) }),
                    },
                ],
            }],
        };
        let html = to_html(&doc);
        assert!(html.contains("a&lt;b&amp;&quot;"));
        assert!(html.contains("<title>&lt;t&gt;</title>"));
        assert!(!html.contains("script") && !html.contains("<line"), "an invalid colour draws nothing");
        assert!(html.contains(r##"<rect x="1.00" y="2.00" width="3.00" height="4.00" fill="#abcdef" stroke="#123" stroke-width="0.50"/>"##));
    }
}
