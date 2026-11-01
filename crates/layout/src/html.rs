//! HTML writer: one absolutely positioned box per item, one sheet per page, printable as is.

use std::fmt::Write as _;

use reportcraft_model::Align;

use crate::Document;

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
        for it in &page.items {
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
                items: vec![Item { x_mm: 0.0, y_mm: 0.0, width_mm: 1.0, height_mm: 1.0, style: TextStyle::default(), text: "a<b&\"".into() }],
            }],
        };
        let html = to_html(&doc);
        assert!(html.contains("a&lt;b&amp;&quot;"));
        assert!(html.contains("<title>&lt;t&gt;</title>"));
    }
}
