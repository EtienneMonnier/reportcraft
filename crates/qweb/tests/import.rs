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
            Content::Subreport { .. } => "Subreport".into(),
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
    // Each document numbers its own pages, as Odoo prints them.
    assert!(group.restart_page_numbers);

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
    // A calculation on fields is a formula, an amount with two decimals.
    assert_eq!(detail, ["[line.product_id.name]", "[line.quantity]", "=[line.price_unit] * 1.2"]);
    assert_eq!(r.detail.elements[1].style.align, Align::Right);
    assert_eq!(r.detail.elements[2].format, Some(Format::Number { decimals: 2, grouping: true }));
    // Pieces on a right-aligned line are joined in one formula; amounts keep two decimals.
    let footer = texts(&group.footer.elements);
    assert_eq!(footer, ["=\"Total: \" & TEXT([o.amount_total], \"#,##0.00\")", "Delivered", "Cancelled", "To deliver"]);
    assert_eq!(group.footer.elements[0].style.align, Align::Right);
    assert!(group.footer.elements[0].style.bold);
    // As the HTML does, the bands give back the room of hidden branches, and text values that end
    // a line wrap; amounts don't.
    assert!(group.header.can_shrink && group.footer.can_shrink && r.detail.can_shrink);
    assert!(name.can_grow && group.header.elements[5].can_grow && r.detail.elements[0].can_grow);
    assert!(!r.detail.elements[2].can_grow && !group.header.elements[0].can_grow);
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
    let out = import(&DELIVERY.replace("line.price_unit * 1.2", "line.price_unit * o.rate()"), None).unwrap();
    let messages: Vec<String> = out.diagnostics.iter().map(|d| format!("{}: {}", d.location, d.message)).collect();
    let has = |needle: &str| messages.iter().any(|m| m.contains(needle));
    assert!(has("the file holds 2 templates"), "{messages:?}");
    assert!(has("line 15: `t-set=\"lines\"`: filters and sorts aren't applied, `lines` reads every record of `o.line_ids`"), "{messages:?}");
    assert!(!has("web.external_layout"), "the letterhead is converted: {messages:?}");
    assert!(messages.iter().any(|m| m.starts_with("line 20: expression `line.price_unit * o.rate()`")), "{messages:?}");
    assert!(out.diagnostics.iter().all(|d| d.severity == Severity::Warning));
}

#[test]
fn sample_data_has_a_row_per_line_of_each_document() {
    let out = import(DELIVERY, None).unwrap();
    let s = &out.sample;
    let names: Vec<&str> = s.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "o.id",
            "o.name",
            "o.partner_id.name",
            "o.note",
            "o.company_id.logo",
            "line.product_id.name",
            "line.quantity",
            "line.price_unit",
            "o.amount_total",
            "o.state",
            // Read by the letterhead.
            "o.company_id.name",
            "o.company_id.street",
            "o.company_id.street2",
            "o.company_id.zip",
            "o.company_id.city",
            "o.company_id.country_id.name",
            "o.company_id.phone",
            "o.company_id.email",
            "o.company_id.website"
        ]
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
    assert_eq!(s.rows[3][col("o.company_id.name")], Value::Text("Company 2".into()));
}

#[test]
fn odoo_page_layouts_become_the_page_header_and_footer() {
    let r = import(DELIVERY, None).unwrap().report;
    // `web.external_layout`: the document's company above, its contacts and page numbers below.
    assert_eq!(
        texts(&r.page_header.elements),
        [
            "image o.company_id.logo",
            "[o.company_id.name]",
            "[o.company_id.street]",
            "[o.company_id.street2]",
            "=TRIM([o.company_id.zip] & \" \" & [o.company_id.city])",
            "[o.company_id.country_id.name]",
            "line"
        ]
    );
    assert!(r.page_header.elements.iter().all(|e| e.x_mm + e.width_mm <= 180.0 + 1e-9 && e.y_mm + e.height_mm <= r.page_header.height_mm));
    assert_eq!(texts(&r.page_footer.elements), ["line", "[o.company_id.phone]", "[o.company_id.email]", "[o.company_id.website]", "PageNumber"]);
    assert!(r.validate().is_ok());

    // `web.internal_layout`: one line, the document variable followed.
    let internal = r#"<t t-name="m.list"><t t-call="web.html_container"><t t-foreach="docs" t-as="doc">
        <t t-call="web.internal_layout"><p t-field="doc.name"/></t></t></t></t>"#;
    let out = import(internal, None).unwrap();
    assert_eq!(texts(&out.report.page_header.elements), ["=TEXT(TODAY(), \"yyyy-mm-dd\")", "[doc.company_id.name]", "PageNumber", "line"]);
    assert!(out.report.page_footer.elements.is_empty());
    assert!(out.sample.column_index("doc.company_id.name").is_some());
    // Both in one template: the first is kept, and that is said.
    let both = r#"<t t-name="x"><t t-foreach="docs" t-as="o"><t t-call="web.internal_layout"/><t t-call="web.external_layout"/></t></t>"#;
    let out = import(both, None).unwrap();
    assert_eq!(out.report.page_header.elements.len(), 4);
    assert!(out.diagnostics.iter().any(|d| d.message.contains("another page layout")));
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

/// An original template with loops inside the document's: serial numbers under each line, payments
/// below the lines, and one more loop in the same band.
const RETURNS: &str = r#"<odoo><template id="report_rental_return">
<t t-call="web.html_container"><t t-foreach="docs" t-as="o">
  <div class="page">
    <h2 t-field="o.name"/>
    <table class="table">
      <thead><tr><th>Item</th><th>Quantity</th></tr></thead>
      <tbody>
        <t t-foreach="o.move_ids" t-as="move">
          <tr><td t-field="move.product_id.name"/><td t-field="move.quantity"/></tr>
          <t t-foreach="move.move_line_ids" t-as="ml">
            <tr t-if="ml.lot_id"><td>Serial <span t-field="ml.lot_id.name"/></td><td t-field="ml.quantity"/></tr>
          </t>
        </t>
        <tr><td>Returned in good order</td></tr>
      </tbody>
    </table>
    <p>Payments</p>
    <div t-foreach="o.payment_ids" t-as="pay"><span t-field="pay.date"/> <span t-field="pay.amount"/></div>
    <p class="text-end">Total: <span t-field="o.amount_total"/></p>
    <div t-foreach="o.note_ids" t-as="n"><span t-field="n.text"/></div>
  </div>
</t></t></template></odoo>"#;

#[test]
fn nested_loops_become_linked_subreports_with_sample_rows() {
    let out = import(RETURNS, None).unwrap();
    let r = &out.report;
    let subreport = |els: &[Element]| -> Option<(String, Vec<String>, Vec<String>, f64)> {
        els.iter().find_map(|e| match &e.content {
            Content::Subreport { report, dataset, links, .. } => {
                Some((dataset.clone(), links.iter().map(|l| format!("{}={}", l.parent, l.child)).collect(), texts(&report.detail.elements), e.y_mm))
            }
            _ => None,
        })
    };
    // Under each line, its serial numbers: a subreport linked to the document and the line.
    assert_eq!(texts(&r.detail.elements), ["[move.product_id.name]", "[move.quantity]", "Subreport"]);
    let (name, links, rows, y) = subreport(&r.detail.elements).unwrap();
    assert_eq!(name, "move_move_line_ids");
    assert_eq!(links, ["o.id=o.id", "move.id=move.id"]);
    assert_eq!(rows, ["Serial ", "[ml.lot_id.name]", "[ml.quantity]"]);
    assert!(y >= 5.0, "below the line's row: {y}");
    let Some(Content::Subreport { report: serials, .. }) = r.detail.elements.last().map(|e| &e.content) else { panic!() };
    // The row's condition hides the whole row (a row without serial number takes no room).
    assert_eq!(serials.detail.visible_if.as_deref(), Some("NOT(ISBLANK([ml.lot_id]))"));
    assert!(serials.detail.elements.iter().all(|e| e.visible_if.is_none()));
    // The row after the loops is the group footer's; payments are the footer's subreport, the
    // total after them (the layout moves it below the payments).
    let footer = texts(&r.groups[0].footer.elements);
    assert_eq!(footer[0], "Returned in good order");
    let (name, links, rows, y) = subreport(&r.groups[0].footer.elements).unwrap();
    assert_eq!((name.as_str(), links), ("o_payment_ids", vec!["o.id=o.id".to_owned()]));
    assert_eq!(rows, ["[pay.date]", "[pay.amount]"]);
    let total =
        r.groups[0].footer.elements.iter().find(|e| matches!(&e.content, Content::Formula { expression } if expression.contains("Total"))).unwrap();
    assert!(total.y_mm > y);
    // One loop per band: the next is shown once, and said.
    assert!(footer.contains(&"{n.text}".to_owned()), "{footer:?}");
    let messages: Vec<&str> = out.diagnostics.iter().map(|d| d.message.as_str()).collect();
    assert!(messages.contains(&"one loop per band is repeated as a subreport"), "{messages:?}");
    assert!(!messages.iter().any(|m| m.contains("move_line_ids") || m.contains("payment_ids")), "{messages:?}");

    // The main rows know the line's id, not the nested loops' columns.
    let names: Vec<&str> = out.sample.columns.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"move.id") && !names.iter().any(|n| n.starts_with("ml.") || n.starts_with("pay.")), "{names:?}");
    // Each nested loop has its sample rows: two per line, or per document.
    let data: Vec<(&str, Vec<&str>, usize)> =
        out.datasets.iter().map(|(n, rows)| (n.as_str(), rows.columns.iter().map(|c| c.name.as_str()).collect(), rows.rows.len())).collect();
    assert_eq!(
        data,
        [
            ("move_move_line_ids", vec!["o.id", "move.id", "ml.lot_id.name", "ml.quantity", "ml.lot_id"], 12),
            ("o_payment_ids", vec!["o.id", "pay.date", "pay.amount"], 4)
        ]
    );
    let (_, serial_rows) = &out.datasets[0];
    // Document 2, line 3, its second serial number.
    assert_eq!(serial_rows.rows[11][..3], [Value::Int(2), Value::Int(3), Value::Text("Lot 2".into())]);
    let mut columns = std::collections::BTreeMap::new();
    for (n, rows) in &out.datasets {
        columns.insert(n.clone(), rows.columns.clone());
    }
    // Valid against the sample rows and the nested loops' rows, links included.
    let problems = r.check_with_datasets(Some(&out.sample.columns), &columns);
    assert!(problems.is_empty(), "{problems:?}");
}

/// An original quotation template in the shape of Odoo 20's documents: title lines (sections,
/// subsections, notes) mixed with the products, named conditions, alternatives inside a row and a
/// cell, a cell for the screen only, and the section's total computed in Python.
const QUOTE: &str = r#"<odoo><template id="report_quote">
<t t-call="web.html_container"><t t-foreach="docs" t-as="o">
  <div class="page">
    <table class="table">
      <thead><tr>
        <th>Item</th><th class="text-end">Quantity</th>
        <th t-attf-class="text-end {{ 'd-none d-md-table-cell' if report_type == 'html' else '' }}">Price</th>
        <th class="text-end">Amount</th>
      </tr></thead>
      <tbody>
        <t t-foreach="o.quote_line_ids" t-as="line">
          <t t-set="is_title" t-value="line.display_type in ('line_section', 'line_subsection')"/>
          <t t-set="is_note" t-value="line.display_type == 'line_note'"/>
          <tr>
            <t t-if="not line.display_type or line.display_type == 'product'">
              <td><span t-field="line.name"/></td>
              <td class="text-end"><span t-field="line.quantity"/></td>
              <td class="text-end"><span t-field="line.price_unit"/></td>
              <td class="text-end">
                <span t-if="o.tax_mode == 'excluded'" t-field="line.price_subtotal"/>
                <span t-if="o.tax_mode == 'included'" t-field="line.price_total"/>
              </td>
            </t>
            <t t-elif="is_title">
              <t t-set="title_total" t-value="line.compute_section_total()"/>
              <td t-att-colspan="3 if o.tax_mode else 2"><span t-field="line.name"/></td>
              <td t-attf-class="{{ 'd-table-cell' if report_type == 'html' else 'd-none' }}">For the screen</td>
              <td class="text-end"><span t-out="title_total"/></td>
            </t>
            <t t-elif="is_note">
              <td colspan="99"><em t-field="line.name"/></td>
            </t>
          </tr>
        </t>
      </tbody>
    </table>
  </div>
</t></t></template></odoo>"#;

#[test]
fn title_lines_group_the_lines_by_section() {
    let out = import(QUOTE, None).unwrap();
    let r = &out.report;
    let at = |els: &[Element]| -> Vec<(String, f64, Option<String>)> {
        texts(els).into_iter().zip(els).map(|(t, e)| (t, e.x_mm, e.visible_if.clone())).collect()
    };
    // Grouped by document, then section, then subsection.
    let fields: Vec<&str> = r.groups.iter().map(|g| g.field.as_str()).collect();
    assert_eq!(fields, ["o.id", "line.section_number", "line.subsection_number"]);
    for (group, kind) in r.groups[1..].iter().zip(["line_section", "line_subsection"]) {
        // The title's row is the header, shown when the group starts with a title line, bold.
        assert_eq!(group.header.visible_if.as_deref(), Some(format!("[line.display_type] = \"{kind}\"").as_str()));
        let header = at(&group.header.elements);
        assert_eq!(
            header.iter().map(|(t, x, _)| (t.as_str(), *x)).collect::<Vec<_>>(),
            [("[line.name]", 0.5), ("=SUM([line.price_subtotal])", 135.5)]
        );
        assert!(group.header.elements.iter().all(|e| e.style.bold));
        assert!(group.footer.elements.is_empty());
    }
    // Title lines take no room among the lines.
    assert_eq!(r.detail.visible_if.as_deref(), Some("AND([line.display_type] <> \"line_section\", [line.display_type] <> \"line_subsection\")"));
    // The product's cells, the title's and the note's start at the same column; the screen-only
    // cell is left out, the computed colspan fills the row up to the total's column, and the two
    // amounts are alternatives laid over each other.
    let detail = at(&r.detail.elements);
    let cells: Vec<(&str, f64)> = detail.iter().map(|(t, x, _)| (t.as_str(), *x)).collect();
    assert_eq!(
        cells,
        [
            ("[line.name]", 0.5),
            ("[line.quantity]", 45.5),
            ("[line.price_unit]", 90.5),
            ("[line.price_subtotal]", 135.5),
            ("[line.price_total]", 135.5),
            ("[line.name]", 0.5),
            ("=SUM([line.price_subtotal])", 135.5),
            ("[line.name]", 0.5),
        ]
    );
    assert_eq!(
        r.detail.elements[4].visible_if.as_deref(),
        Some("AND(OR(ISBLANK([line.display_type]), [line.display_type] = \"product\"), [o.tax_mode] = \"included\")")
    );
    assert_eq!(r.detail.elements[2].style.align, Align::Right);
    assert!(detail[7].2.as_deref().is_some_and(|c| c.ends_with("[line.display_type] = \"line_note\")")), "{detail:?}");
    let messages: Vec<&str> = out.diagnostics.iter().map(|d| d.message.as_str()).collect();
    assert!(messages.contains(&"`title_total` is read as the sum of `line.price_subtotal` over the section's lines"), "{messages:?}");
    assert!(!messages.iter().any(|m| m.contains("is_title") || m.contains("is_note") || m.contains("beyond")), "{messages:?}");

    // Sample rows: a title line then products in section 1, compared fields at their first value.
    let column = |name: &str| out.sample.columns.iter().position(|c| c.name == name).unwrap();
    let (kind, number, sub, mode, amount) = (
        column("line.display_type"),
        column("line.section_number"),
        column("line.subsection_number"),
        column("o.tax_mode"),
        column("line.price_subtotal"),
    );
    assert_eq!(out.sample.columns[number].field_type, FieldType::Integer);
    let row = |i: usize, c: usize| out.sample.rows[i][c].clone();
    assert_eq!((row(0, kind), row(1, kind)), (Value::Text("line_section".into()), Value::Text("product".into())));
    assert_eq!((row(0, number), row(2, number), row(2, sub)), (Value::Int(1), Value::Int(1), Value::Int(0)));
    assert_eq!((row(0, amount), row(1, amount)), (Value::Null, Value::Number(25.0)));
    assert_eq!(row(0, mode), Value::Text("excluded".into()));
    let problems = r.check_with_datasets(Some(&out.sample.columns), &Default::default());
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn records_joined_into_a_text_read_as_one_column() {
    let xml = r#"<odoo><template id="report_bill"><t t-foreach="docs" t-as="o"><table>
        <thead><tr><th>Item</th><th>Taxes</th><th>Lots</th></tr></thead>
        <tbody><t t-foreach="o.line_ids" t-as="line">
          <tr>
            <t t-set="taxes" t-value="', '.join(tax.tax_label for tax in line.tax_ids if tax.tax_label)"/>
            <td t-field="line.name"/>
            <td><span t-out="taxes"/></td>
            <td t-out="' / '.join(line.lot_ids.mapped('name'))"/>
          </tr>
        </t></tbody></table>
        <p>All taxes: <span t-out="', '.join(o.tax_ids.mapped('name'))"/></p>
    </t></template></odoo>"#;
    let out = import(xml, None).unwrap();
    assert_eq!(texts(&out.report.detail.elements), ["[line.name]", "[line.tax_ids.tax_label]", "[line.lot_ids.name]"]);
    // Text, not an amount, though the name speaks of taxes.
    assert!(out.report.detail.elements.iter().all(|e| e.format.is_none()));
    let footer = texts(&out.report.groups[0].footer.elements);
    assert!(footer.contains(&"[o.tax_ids.name]".to_owned()), "{footer:?}");
    let column = |name: &str| out.sample.columns.iter().find(|c| c.name == name).map(|c| c.field_type);
    assert_eq!(column("line.tax_ids.tax_label"), Some(FieldType::Text));
    assert_eq!(column("o.tax_ids.name"), Some(FieldType::Text));
    let messages: Vec<&str> = out.diagnostics.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(
        messages,
        [
            "`line.tax_ids.tax_label` holds the values of the records joined with \", \": the query joins them (`string_agg`)",
            "`line.lot_ids.name` holds the values of the records joined with \" / \": the query joins them (`string_agg`)",
            "`o.tax_ids.name` holds the values of the records joined with \", \": the query joins them (`string_agg`)",
        ]
    );
}

#[test]
fn calculations_and_comparisons_between_fields_become_formulas() {
    let xml = r#"<odoo><template id="report_delivery"><t t-foreach="docs" t-as="o"><table>
        <thead><tr><th>Item</th><th>Left</th><th>Rate</th></tr></thead>
        <tbody><tr t-foreach="o.move_ids" t-as="move">
          <td t-field="move.product_id.name"/>
          <td t-if="move.quantity != move.product_uom_qty"><span t-esc="move.product_uom_qty - move.quantity"/></td>
          <td><span t-esc="move.weight * company.rate"/></td>
        </tr></tbody></table>
        <p>Paid: <span t-esc="round(o.amount_total - o.amount_residual, 2)"/></p>
    </t></template></odoo>"#;
    let out = import(xml, None).unwrap();
    let r = &out.report;
    assert_eq!(texts(&r.detail.elements), ["[move.product_id.name]", "=[move.product_uom_qty] - [move.quantity]", "{move.weight * company.rate}"]);
    let left = &r.detail.elements[1];
    assert_eq!(left.visible_if.as_deref(), Some("[move.quantity] <> [move.product_uom_qty]"));
    // Quantities aren't amounts: no forced decimals.
    assert_eq!(left.format, None);
    let footer = texts(&r.groups[0].footer.elements);
    assert_eq!(footer, ["Paid: ", "=ROUND([o.amount_total] - [o.amount_residual], 2)"]);
    assert_eq!(r.groups[0].footer.elements[1].format, Some(Format::Number { decimals: 2, grouping: true }));
    // `company` isn't a field of the documents: said, and shown as written.
    let messages: Vec<&str> = out.diagnostics.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(messages, ["expression `move.weight * company.rate` isn't converted: shown as written"]);
    let names: Vec<&str> = out.sample.columns.iter().map(|c| c.name.as_str()).collect();
    for n in ["move.quantity", "move.product_uom_qty", "o.amount_total", "o.amount_residual"] {
        assert!(names.contains(&n), "{names:?}");
    }
    let problems = r.check_with_datasets(Some(&out.sample.columns), &Default::default());
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn the_import_describes_its_data_for_a_query() {
    let xml = r#"<odoo>
<record id="action_other" model="ir.actions.report">
  <field name="model">res.partner</field><field name="report_name">test.report_other</field>
</record>
<record id="action_quote" model="ir.actions.report">
  <field name="model">sale.order</field><field name="report_name">test.report_quote</field>
</record>
<template id="report_quote"><t t-foreach="docs" t-as="o">
  <p t-field="o.name"/>
  <t t-foreach="o.order_line" t-as="line">
    <p><span t-field="line.name"/> <span t-out="' / '.join(line.tax_ids.mapped('name'))"/></p>
    <t t-foreach="line.lot_ids" t-as="lot"><p t-field="lot.name"/></t>
  </t>
</t></template></odoo>"#;
    let shape = import(xml, None).unwrap().shape;
    assert_eq!(shape.model.as_deref(), Some("sale.order"), "the action whose report name is the template");
    assert_eq!(shape.doc_var.as_deref(), Some("o"));
    assert_eq!(shape.lines, Some(("line".to_owned(), "o.order_line".to_owned())));
    assert_eq!(shape.joined, [("line.tax_ids.name".to_owned(), " / ".to_owned())]);
    assert!(shape.columns.iter().any(|c| c == "o.id") && shape.columns.iter().any(|c| c == "line.name"), "{:?}", shape.columns);
    let lots = &shape.datasets[0];
    assert_eq!((lots.var.as_str(), lots.source.as_str(), lots.under_line), ("lot", "line.lot_ids", true));
    assert!(lots.columns.iter().any(|c| c == "lot.name"), "{:?}", lots.columns);
}

/// An original order template in the shape Odoo's layout expects: the title and the addresses set
/// as blocks for `web.external_layout`, a grid row of dates, a title line chosen by state.
const ORDER: &str = r#"<t t-name="shop.report_order">
  <t t-foreach="docs" t-as="o">
    <t t-set="layout_document_title">
      <span t-if="o.state == 'draft'">Quotation</span>
      <span t-elif="o.state == 'sale'">Order</span>
      <span t-field="o.name"/>
    </t>
    <t t-set="information_block" t-if="o.partner_shipping_id != o.partner_id">
      <strong>Ship to</strong>
      <div t-field="o.partner_shipping_id" t-options='{"widget": "contact", "fields": ["address", "name"]}'/>
    </t>
    <t t-set="address">
      <address t-field="o.partner_id" t-options='{"widget": "contact", "fields": ["address", "name", "phone"]}'/>
    </t>
    <t t-call="web.external_layout" address="address">
      <div class="row mb-4">
        <div class="col" t-if="o.date_order"><strong>Date</strong><div t-field="o.date_order"/></div>
        <div class="col-3 offset-1"><strong>Reference</strong><div t-field="o.client_order_ref"/></div>
        <div class="col-4 ms-auto"><strong>Salesperson</strong><div t-field="o.user_id.name"/></div>
      </div>
      <p>Thank you.</p>
    </t>
  </t>
</t>"#;

#[test]
fn the_page_layout_shows_the_title_and_addresses_and_grid_rows_sit_side_by_side() {
    let out = import(ORDER, None).unwrap();
    let header = &out.report.groups[0].header;
    let at = |text: &str| {
        header
            .elements
            .iter()
            .find(|e| texts(std::slice::from_ref(e))[0] == text)
            .unwrap_or_else(|| panic!("{text} in {:?}", texts(&header.elements)))
    };
    // The information block on the left half, under its condition; the address on the right.
    let ship = at("Ship to");
    assert!(ship.x_mm < 1.0 && ship.width_mm <= 90.0 + 1e-9);
    assert_eq!(ship.visible_if.as_deref(), Some("[o.partner_shipping_id] <> [o.partner_id]"));
    let street = at("[o.partner_id.street]");
    assert!((street.x_mm - 105.0).abs() < 0.11 && (street.width_mm - 75.0).abs() < 0.11, "{street:?}");
    // The contact widget: the name with its company's, then a line per part of the address.
    let name = at(
        "=IF(OR(ISBLANK([o.partner_id.parent_id.name]), [o.partner_id.parent_id.name] = [o.partner_id.name]), [o.partner_id.name], [o.partner_id.parent_id.name] & \", \" & [o.partner_id.name])",
    );
    assert!(name.y_mm < street.y_mm && name.y_mm == ship.y_mm);
    for part in [
        "[o.partner_id.street2]",
        "=TRIM([o.partner_id.zip] & \" \" & [o.partner_id.city])",
        "[o.partner_id.country_id.name]",
        "[o.partner_id.phone]",
    ] {
        assert!(at(part).y_mm > street.y_mm, "{part}");
    }
    assert!(header.elements.iter().all(|e| !texts(std::slice::from_ref(e))[0].contains("email")));
    // The title below both, a heading whose words show under their own conditions.
    let title = at(
        "=TRIM(IF([o.state] = \"draft\", \"Quotation \", \"\") & IF(AND(NOT([o.state] = \"draft\"), [o.state] = \"sale\"), \"Order \", \"\") & [o.name])",
    );
    assert!(title.style.bold && title.style.size_pt == 16.0 && title.visible_if.is_none());
    assert!(title.y_mm > at("[o.partner_shipping_id.country_id.name]").y_mm);
    // The grid row: columns side by side from the same top, as wide as their classes say.
    let (date, reference, seller) = (at("Date"), at("Reference"), at("Salesperson"));
    assert!(date.y_mm > title.y_mm && date.y_mm == reference.y_mm && date.y_mm == seller.y_mm);
    assert!(date.x_mm < 1.0 && (date.width_mm - 60.0).abs() < 0.11, "a plain col shares the rest: {date:?}");
    assert!((reference.x_mm - 75.0).abs() < 0.11 && (reference.width_mm - 45.0).abs() < 0.11, "{reference:?}");
    assert!((seller.x_mm - 120.0).abs() < 0.11 && (seller.x_mm + seller.width_mm - 180.0).abs() < 0.11, "{seller:?}");
    assert_eq!(at("[o.date_order]").visible_if.as_deref(), Some("NOT(ISBLANK([o.date_order]))"));
    let thanks = at("Thank you.");
    assert!(thanks.y_mm > at("[o.user_id.name]").y_mm && thanks.x_mm < 1.0);
    assert!(!out.diagnostics.iter().any(|d| d.message.contains("t-set")), "{:?}", out.diagnostics);
    let problems: Vec<_> = out.report.check(Some(&out.sample.columns)).into_iter().filter(|d| d.severity == Severity::Error).collect();
    assert!(problems.is_empty(), "{problems:?}");
}

/// An original invoice template in the shape of Odoo's: the totals called from a template of the
/// same file inside the totals table (through a primary copy of it), a loop over the one subtotal
/// and its tax groups, payments under a flag the printed template doesn't set, the totals floated
/// right.
const TOTALS: &str = r#"<odoo>
<template id="report_bill">
  <t t-call="web.html_container"><t t-foreach="docs" t-as="o">
    <h2><span t-if="not proforma">Bill</span><span t-else="">Pro forma</span></h2>
    <table class="table"><tbody>
      <tr t-foreach="o.line_ids" t-as="line"><td t-field="line.name"/><td class="text-end" t-field="line.price_subtotal"/></tr>
    </tbody></table>
    <div class="float-end">
      <table class="table">
        <t t-if="o.tax_totals" t-call="shop.tax_totals" tax_totals="o.tax_totals"/>
        <t t-if="print_with_payments">
          <t t-set="payments_vals" t-value="o.sudo().invoice_payments_widget['content']"/>
          <t t-foreach="payments_vals" t-as="payment_vals">
            <tr t-if="payment_vals['is_exchange'] == 0"><td>Paid</td><td t-out="payment_vals['amount']"/></tr>
          </t>
        </t>
      </table>
    </div>
    <p t-if="o.tax_totals.get('display_in_company_currency')">In the company's currency</p>
  </t></t>
</template>
<template id="tax_totals_template">
  <t t-foreach="tax_totals['subtotals']" t-as="subtotal">
    <tr><td t-out="subtotal['name']"/><td class="text-end" t-out="subtotal['base_amount_currency']"/></tr>
    <t t-foreach="subtotal['tax_groups']" t-as="tax_group">
      <tr>
        <t t-if="tax_group['same']"><td t-out="tax_group['group_name']"/><td class="text-end" t-out="tax_group['tax_amount_currency']"/></t>
        <t t-else=""><td>on base</td><td class="text-end" t-out="tax_group['tax_amount_currency']"/></t>
      </tr>
    </t>
  </t>
  <tr t-if="'cash_rounding' in tax_totals"><td>Rounding</td><td t-out="tax_totals['cash_rounding']"/></tr>
  <tr><td>Total</td><td class="text-end" t-out="tax_totals['total_amount_currency']"/></tr>
</template>
<template id="tax_totals" inherit_id="shop.tax_totals_template" primary="True"></template>
</odoo>"#;

#[test]
fn called_totals_single_subtotals_and_unset_flags_read_as_odoo_prints_them() {
    let out = import(TOTALS, Some("report_bill")).unwrap();
    let r = &out.report;
    let footer = &r.groups[0].footer;
    let shown: Vec<(String, f64, Option<String>)> =
        texts(&footer.elements).into_iter().zip(&footer.elements).map(|(t, e)| (t, e.x_mm, e.visible_if.clone())).collect();
    let find = |t: &str| shown.iter().find(|(s, ..)| s == t).unwrap_or_else(|| panic!("{t} in {shown:?}"));
    // The called template's rows, the call's names read through: the one subtotal is the
    // document's, under the call's condition; the totals are in the right half, two columns.
    let (_, x, cond) = find("[o.tax_totals.subtotals.name]");
    assert!((x - 90.5).abs() < 0.11, "{x}");
    assert_eq!(cond.as_deref(), Some("NOT(ISBLANK([o.tax_totals]))"));
    assert!((find("[o.tax_totals.subtotals.base_amount_currency]").1 - 135.5).abs() < 0.11);
    assert!((find("[o.tax_totals.total_amount_currency]").1 - 135.5).abs() < 0.11);
    assert_eq!(find("Rounding").2.as_deref(), Some("AND(NOT(ISBLANK([o.tax_totals])), NOT(ISBLANK([o.tax_totals.cash_rounding])))"));
    assert_eq!(find("In the company's currency").2.as_deref(), Some("[o.tax_totals.display_in_company_currency]"));
    // The tax groups: a nested loop over the document, its subreport in the right half.
    let sub = footer.elements.iter().find(|e| matches!(e.content, Content::Subreport { .. })).unwrap();
    assert!((sub.x_mm - 90.0).abs() < 0.11 && (sub.width_mm - 90.0).abs() < 0.11, "{sub:?}");
    let Content::Subreport { report: groups, dataset, .. } = &sub.content else { panic!() };
    assert_eq!(dataset, "o_tax_totals_subtotals_tax_groups");
    let rows = texts(&groups.detail.elements);
    assert!(rows.contains(&"[tax_group.group_name]".to_owned()) && rows.contains(&"on base".to_owned()), "{rows:?}");
    assert!(groups.detail.elements.iter().all(|e| e.x_mm < 90.0 + 1e-9), "laid out in the subreport's own width");
    // A flag nothing sets is false, said once; the payments under it never read their Python
    // list's items as fields.
    assert_eq!(find("Paid").2.as_deref(), Some("FALSE"));
    let says = |text: &str| out.diagnostics.iter().filter(|d| d.message.contains(text)).count();
    assert_eq!(says("`print_with_payments` isn't set by the template: read as false"), 1);
    assert_eq!(says("`proforma` isn't set"), 1);
    assert!(!out.sample.columns.iter().any(|c| c.name.starts_with("payment_vals")), "{:?}", out.sample.columns);
    let header = texts(&r.groups[0].header.elements);
    assert_eq!(header, ["=TRIM(IF(NOT(FALSE), \"Bill\", \"\") & IF(NOT(NOT(FALSE)), \"Pro forma\", \"\"))"]);
}

/// An original template with a row of plain columns shown under conditions, a fixed column after
/// them, a row inside a column, and a row of one column.
const DATES: &str = r#"<t t-name="shop.report_dates">
  <t t-foreach="docs" t-as="o">
    <div class="row">
      <div class="col" t-if="o.date"><strong>Date</strong><div t-field="o.date"/></div>
      <div class="col" t-if="o.origin"><strong>Origin</strong><div t-field="o.origin"/></div>
      <div class="col"><div class="row"><div class="col-6">Left</div><div class="col-6" t-if="o.ref">Right</div></div></div>
      <div class="col-3 offset-1"><strong>Seller</strong><div t-field="o.user_id.name"/></div>
    </div>
    <div class="row"><div class="col-6"><span t-field="o.note"/></div></div>
  </t>
</t>"#;

#[test]
fn grid_columns_are_boxes_that_give_their_width_up_when_hidden() {
    let out = import(DATES, None).unwrap();
    let header = &out.report.groups[0].header;
    let flow = |text: &str| {
        let el = header.elements.iter().find(|e| texts(std::slice::from_ref(e))[0] == text).unwrap_or_else(|| panic!("{text}"));
        el.flow.map(|f| (f.row, f.x_mm, f.width_mm, f.grows))
    };
    // Plain columns share 8 twelfths of 180 mm and grow; the fixed one keeps its 3 after 1.
    let row = flow("Date").map_or(0, |f| f.0);
    assert_eq!(flow("Date"), Some((row, 0.0, 40.0, true)));
    assert_eq!(flow("[o.date]"), flow("Date"));
    assert_eq!(flow("Origin"), Some((row, 40.0, 40.0, true)));
    assert_eq!(flow("[o.user_id.name]"), Some((row, 135.0, 45.0, false)));
    // A row inside a column moves with its column; a row of one column is no flow.
    assert_eq!(flow("Left"), Some((row, 80.0, 40.0, true)));
    assert_eq!(flow("Right"), flow("Left"));
    assert_eq!(flow("[o.note]"), None);
    let date = header.elements.iter().find(|e| texts(std::slice::from_ref(e))[0] == "Date").unwrap();
    assert_eq!(date.visible_if.as_deref(), Some("NOT(ISBLANK([o.date]))"));
    // A row with a column pushed right (`ms-auto`) keeps its place.
    let order = import(ORDER, None).unwrap();
    assert!(order.report.groups[0].header.elements.iter().all(|e| e.flow.is_none()));
    let problems: Vec<_> = out.report.check(Some(&out.sample.columns)).into_iter().filter(|d| d.severity == Severity::Error).collect();
    assert!(problems.is_empty(), "{problems:?}");
}
