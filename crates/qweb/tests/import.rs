use reportcraft_core::{FieldType, Value};
use reportcraft_model::{Align, Content, Element, Format, Severity};
use reportcraft_qweb::import;

/// An original template in the shape Odoo documents for reports: a wrapper looping over `docs`
/// that calls the document template of the same file.
const DELIVERY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<odoo>
  <template id="report_delivery_note_document">
    <t t-call="web.external_layout">
      <div class="page">
        <h2>Delivery note <span t-field="o.name"/></h2>
        <p>Customer: <span t-field="o.partner_id.name"/></p>
        <p t-if="o.note">Note: <span t-esc="o.note"/></p>
        <img t-att-src="image_data_uri(o.company_id.logo)" style="max-height: 60px;"/>
        <table class="table table-sm">
          <thead>
            <tr><th>Product</th><th class="text-end">Quantity</th><th class="text-end">Price</th></tr>
          </thead>
          <tbody>
            <t t-set="lines" t-value="o.line_ids.filtered(lambda l: l.quantity)"/>
            <t t-foreach="lines" t-as="line">
              <tr>
                <td><span t-field="line.product_id.name"/></td>
                <td class="text-end"><span t-field="line.quantity"/></td>
                <td class="text-end"><span t-esc="line.price_unit * 1.2"/></td>
              </tr>
            </t>
          </tbody>
        </table>
        <p class="text-end"><strong>Total: <span t-field="o.amount_total"/></strong></p>
        <p t-if="o.state == 'done'">Delivered</p>
        <p t-elif="o.state == 'cancel'">Cancelled</p>
        <p t-else="">To deliver</p>
      </div>
    </t>
  </template>
  <template id="report_delivery_note">
    <t t-call="web.html_container">
      <t t-foreach="docs" t-as="o">
        <t t-set="lang" t-value="o.partner_id.lang"/>
        <t t-call="stock_extra.report_delivery_note_document"/>
      </t>
    </t>
  </template>
</odoo>
"#;

fn texts(els: &[Element]) -> Vec<String> {
    els.iter()
        .map(|e| match &e.content {
            Content::Text { text } => text.clone(),
            Content::Field { field } => format!("[{field}]"),
            Content::Formula { expression } => format!("={expression}"),
            Content::Image { field, .. } => format!("image {}", field.as_deref().unwrap_or("")),
            Content::Line { .. } => "line".into(),
            other => format!("{other:?}"),
        })
        .collect()
}

#[test]
fn a_document_loop_becomes_a_group_and_its_lines_the_detail() {
    let out = import(DELIVERY, None).unwrap();
    let r = &out.report;
    assert_eq!(r.title, "report_delivery_note");
    assert_eq!(out.templates, ["report_delivery_note_document", "report_delivery_note"]);
    assert_eq!(r.groups.len(), 1);
    let group = &r.groups[0];
    assert_eq!(group.field, "o.id");
    assert!(group.page_break_before);

    let header = texts(&group.header.elements);
    assert_eq!(
        header,
        [
            "Delivery note ",
            "[o.name]",
            "Customer: ",
            "[o.partner_id.name]",
            "Note: ",
            "[o.note]",
            "image o.company_id.logo",
            "Product",
            "Quantity",
            "Price",
            "line"
        ]
    );
    // The heading is large and bold; the field after the label takes the rest of the line.
    let name = &group.header.elements[1];
    assert!(name.style.bold && name.style.size_pt == 16.0);
    assert!((name.x_mm + name.width_mm - 180.0).abs() < 0.11, "{name:?}");
    // `t-if` on a paragraph shows its elements only when the field has a value.
    assert_eq!(group.header.elements[4].visible_if.as_deref(), Some("NOT(ISBLANK([o.note]))"));
    assert_eq!(group.header.elements[5].visible_if.as_deref(), Some("NOT(ISBLANK([o.note]))"));
    // Table header cells: three equal columns, bold, aligned as their classes say.
    let quantity = &group.header.elements[8];
    assert!(quantity.style.bold && quantity.style.align == Align::Right);
    assert!((quantity.x_mm - 60.5).abs() < 0.11 && (quantity.width_mm - 59.0).abs() < 0.11, "{quantity:?}");

    let detail = texts(&r.detail.elements);
    assert_eq!(detail, ["[line.product_id.name]", "[line.quantity]", "{line.price_unit * 1.2}"]);
    assert_eq!(r.detail.elements[1].style.align, Align::Right);
    // Pieces on a right-aligned line are joined in one formula; amounts keep two decimals.
    let footer = texts(&group.footer.elements);
    assert_eq!(footer, ["=\"Total: \" & TEXT([o.amount_total], \"#,##0.00\")", "Delivered", "Cancelled", "To deliver"]);
    assert_eq!(group.footer.elements[0].style.align, Align::Right);
    assert!(group.footer.elements[0].style.bold);
    // `t-elif` and `t-else` show when the branches before them don't.
    let conditions: Vec<_> = group.footer.elements[1..].iter().map(|e| e.visible_if.as_deref().unwrap_or("")).collect();
    assert_eq!(
        conditions,
        [
            "[o.state] = \"done\"",
            "AND(NOT([o.state] = \"done\"), [o.state] = \"cancel\")",
            "AND(NOT([o.state] = \"done\"), NOT([o.state] = \"cancel\"))"
        ]
    );

    // The report is valid against its sample data's columns.
    let problems: Vec<_> = r.check(Some(&out.sample.columns)).into_iter().filter(|d| d.severity == Severity::Error).collect();
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn what_isnt_converted_is_reported_by_line() {
    let out = import(DELIVERY, None).unwrap();
    let messages: Vec<String> = out.diagnostics.iter().map(|d| format!("{}: {}", d.location, d.message)).collect();
    let has = |needle: &str| messages.iter().any(|m| m.contains(needle));
    assert!(has("the file holds 2 templates"), "{messages:?}");
    assert!(has("line 15: `t-set=\"lines\"`: filters and sorts aren't applied, `lines` reads every record of `o.line_ids`"), "{messages:?}");
    assert!(has("web.external_layout"), "{messages:?}");
    assert!(messages.iter().any(|m| m.starts_with("line 20: expression `line.price_unit * 1.2`")), "{messages:?}");
    assert!(out.diagnostics.iter().all(|d| d.severity == Severity::Warning));
}

#[test]
fn sample_data_has_a_row_per_line_of_each_document() {
    let out = import(DELIVERY, None).unwrap();
    let s = &out.sample;
    let names: Vec<&str> = s.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        ["o.id", "o.name", "o.partner_id.name", "o.note", "o.company_id.logo", "line.product_id.name", "line.quantity", "o.amount_total", "o.state"]
    );
    assert_eq!(s.rows.len(), 6);
    let col = |n: &str| s.column_index(n).unwrap();
    assert_eq!(s.columns[col("line.quantity")].field_type, FieldType::Number);
    assert_eq!(s.rows[0][col("o.id")], Value::Int(1));
    assert_eq!(s.rows[3][col("o.id")], Value::Int(2));
    assert_eq!(s.rows[1][col("o.partner_id.name")], Value::Text("Partner 1".into()));
    assert_eq!(s.rows[2][col("line.product_id.name")], Value::Text("Product 3".into()));
    assert_eq!(s.rows[4][col("o.amount_total")], Value::Number(250.0));
    assert_eq!(s.rows[0][col("o.company_id.logo")], Value::Null);
}

#[test]
fn a_template_is_chosen_by_name_and_a_single_record_needs_no_loop() {
    let out = import(DELIVERY, Some("report_delivery_note_document")).unwrap();
    // Without the wrapper there is no document loop: one record, a warning, and the line loop
    // isn't recognised (no document variable).
    assert!(out.report.groups.is_empty());
    assert!(out.diagnostics.iter().any(|d| d.message.contains("no `t-foreach=\"docs\"`")));
    assert!(import(DELIVERY, Some("stock_extra.report_delivery_note")).is_ok());
    assert!(import(DELIVERY, Some("nope")).is_err());

    let card = r#"<t t-name="badge"><div class="text-center"><h1 t-field="o.name"/><span>Valid</span></div></t>"#;
    let out = import(card, None).unwrap();
    let els = &out.report.report_header.elements;
    assert_eq!(texts(els), ["[o.name]", "Valid"]);
    assert_eq!(els[0].style.align, Align::Center);
    assert!(els[1].y_mm > els[0].y_mm);
}

#[test]
fn hostile_files_are_refused_or_bounded() {
    assert!(import("", None).is_err());
    assert!(import("<odoo><template id='x'>", None).is_err());
    assert!(import("<odoo/>", None).is_err());
    let entity = r#"<!DOCTYPE x [<!ENTITY a "aaaa">]><t t-name="x">&a;</t>"#;
    assert!(import(entity, None).is_err());
    let big = format!("<t t-name='x'>{}</t>", " ".repeat(reportcraft_qweb::MAX_XML_BYTES));
    assert!(import(&big, None).is_err());
    // Deep nesting and a template calling itself end with warnings, not a crash.
    let deep = format!("<t t-name='x'>{}<span t-field='o.a'/>{}</t>", "<div>".repeat(200), "</div>".repeat(200));
    let out = import(&deep, None).unwrap();
    assert!(out.diagnostics.iter().any(|d| d.message.contains("nested")));
    let looped = r#"<odoo><template id="a"><t t-call="m.b"/></template><template id="b"><p>x</p><t t-call="a"/></template></odoo>"#;
    let out = import(looped, Some("a")).unwrap();
    assert_eq!(texts(&out.report.report_header.elements), ["x"]);
    assert!(out.diagnostics.iter().any(|d| d.message.contains("calls itself")));
    // Many elements stop at the band's limit.
    let many = format!("<t t-name='x'>{}</t>", "<p>a</p>".repeat(2_000));
    let out = import(&many, None).unwrap();
    assert_eq!(out.report.report_header.elements.len(), reportcraft_core::limits::MAX_ELEMENTS_PER_BAND);
    let _ = Format::Number { decimals: 2, grouping: true };
}
