//! An imported QWeb template reading an Odoo-shaped database through the queries the import
//! proposes. Runs only when `REPORTCRAFT_TEST_POSTGRES` holds a connection string to a scratch
//! database (see `docs/pitfalls.md`); otherwise it says so and passes. It creates the schema
//! `rc_odoo_live` (tables named as Odoo's, with made-up rows) and the role `rc_odoo_reader`,
//! whose search path is that schema, and reads through a saved connection as that role.
#![cfg(not(target_arch = "wasm32"))]

use postgres::{Client, NoTls};
use reportcraft_engine::Session;
use serde_json::{Value as Json, json};

fn conninfo() -> Option<String> {
    let c = std::env::var("REPORTCRAFT_TEST_POSTGRES").ok().filter(|s| !s.is_empty());
    if c.is_none() {
        eprintln!("REPORTCRAFT_TEST_POSTGRES not set: live Odoo query test skipped");
    }
    c
}

const SETUP: &str = r#"
DROP SCHEMA IF EXISTS rc_odoo_live CASCADE;
CREATE SCHEMA rc_odoo_live;
SET search_path = rc_odoo_live;
CREATE TABLE ir_model_fields (id serial PRIMARY KEY, model varchar, name varchar, ttype varchar, relation varchar,
  relation_field varchar, relation_table varchar, column1 varchar, column2 varchar, store boolean, related varchar,
  translate varchar, company_dependent boolean);
INSERT INTO ir_model_fields (model, name, ttype, relation, relation_field, relation_table, column1, column2, store, related, translate) VALUES
  ('quote.order', 'name', 'char', NULL, NULL, NULL, NULL, NULL, true, NULL, NULL),
  ('quote.order', 'partner_id', 'many2one', 'res.partner', NULL, NULL, NULL, NULL, true, NULL, NULL),
  ('quote.order', 'line_ids', 'one2many', 'quote.line', 'order_id', NULL, NULL, NULL, true, NULL, NULL),
  ('quote.line', 'order_id', 'many2one', 'quote.order', NULL, NULL, NULL, NULL, true, NULL, NULL),
  ('quote.line', 'sequence', 'integer', NULL, NULL, NULL, NULL, NULL, true, NULL, NULL),
  ('quote.line', 'name', 'text', NULL, NULL, NULL, NULL, NULL, true, NULL, NULL),
  ('quote.line', 'display_type', 'selection', NULL, NULL, NULL, NULL, NULL, true, NULL, NULL),
  ('quote.line', 'quantity', 'float', NULL, NULL, NULL, NULL, NULL, true, NULL, NULL),
  ('quote.line', 'product_id', 'many2one', 'product.product', NULL, NULL, NULL, NULL, true, NULL, NULL),
  ('quote.line', 'tax_ids', 'many2many', 'account.tax', NULL, 'quote_line_tax_rel', 'line_id', 'tax_id', true, NULL, NULL),
  ('product.product', 'product_tmpl_id', 'many2one', 'product.template', NULL, NULL, NULL, NULL, true, NULL, NULL),
  ('product.product', 'name', 'char', NULL, NULL, NULL, NULL, NULL, false, 'product_tmpl_id.name', 'standard'),
  ('product.template', 'name', 'char', NULL, NULL, NULL, NULL, NULL, true, NULL, 'standard'),
  ('account.tax', 'name', 'char', NULL, NULL, NULL, NULL, NULL, true, NULL, 'standard'),
  ('account.tax', 'sequence', 'integer', NULL, NULL, NULL, NULL, NULL, true, NULL, NULL),
  ('res.partner', 'name', 'char', NULL, NULL, NULL, NULL, NULL, true, NULL, NULL);
CREATE TABLE res_partner (id int PRIMARY KEY, name varchar);
CREATE TABLE quote_order (id int PRIMARY KEY, name varchar, partner_id int);
CREATE TABLE product_template (id int PRIMARY KEY, name jsonb);
CREATE TABLE product_product (id int PRIMARY KEY, product_tmpl_id int);
CREATE TABLE account_tax (id int PRIMARY KEY, name jsonb, sequence int);
CREATE TABLE quote_line (id int PRIMARY KEY, order_id int, sequence int, name text, display_type varchar, quantity float8, product_id int);
CREATE TABLE quote_line_tax_rel (line_id int, tax_id int);
INSERT INTO res_partner VALUES (1, 'Partner One'), (2, 'Partner Two');
INSERT INTO quote_order VALUES (1, 'Q1', 1), (2, 'Q2', 2), (3, 'Q3', 1);
INSERT INTO product_template VALUES (1, '{"en_US": "Tent", "fr_FR": "Tente"}'), (2, '{"en_US": "Stove"}');
INSERT INTO product_product VALUES (11, 1), (12, 2);
INSERT INTO account_tax VALUES (1, '{"en_US": "VAT 20%", "fr_FR": "TVA 20 %"}', 2), (2, '{"en_US": "Eco"}', 1);
-- Ids run against the sequence: the order comes from the sequence.
INSERT INTO quote_line VALUES
  (106, 1, 10, 'Camp', 'line_section', NULL, NULL),
  (105, 1, 20, 'Tent line', 'product', 2, 11),
  (104, 1, 30, 'Cooking', 'line_subsection', NULL, NULL),
  (103, 1, 40, 'Stove line', 'product', 1, 12),
  (102, 1, 50, 'Extras', 'line_section', NULL, NULL),
  (101, 1, 60, 'Extra line', 'product', 3, 12),
  (203, 2, 10, 'Loose line', 'product', 1, 11),
  (202, 2, 20, 'Later', 'line_section', NULL, NULL),
  (201, 2, 30, 'Late line', 'product', 1, 12),
  (301, 3, 10, 'Not printed', 'product', 1, 11);
INSERT INTO quote_line_tax_rel VALUES (105, 1), (105, 2), (103, 1);
DO $$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'rc_odoo_reader') THEN CREATE ROLE rc_odoo_reader LOGIN; END IF;
END $$;
ALTER ROLE rc_odoo_reader SET search_path = rc_odoo_live;
GRANT USAGE ON SCHEMA rc_odoo_live TO rc_odoo_reader;
GRANT SELECT ON ALL TABLES IN SCHEMA rc_odoo_live TO rc_odoo_reader;
"#;

/// An original template in the shape of Odoo's documents, with the report action naming its model.
const TEMPLATE: &str = r#"<odoo>
<record id="action_quote" model="ir.actions.report">
  <field name="model">quote.order</field>
  <field name="report_name">test.report_quote</field>
</record>
<template id="report_quote">
<t t-call="web.html_container"><t t-foreach="docs" t-as="o">
  <div class="page">
    <h2>Quote <span t-field="o.name"/></h2>
    <p t-field="o.partner_id"/>
    <table class="table">
      <thead><tr><th>Item</th><th>Product</th><th>Taxes</th><th>Quantity</th></tr></thead>
      <tbody>
        <t t-foreach="o.line_ids" t-as="line">
          <tr>
            <t t-if="line.display_type == 'product'">
              <td><span t-field="line.name"/></td>
              <td><span t-field="line.product_id.name"/></td>
              <td><span t-out="', '.join(line.tax_ids.mapped('name'))"/></td>
              <td><span t-field="line.quantity"/></td>
            </t>
            <t t-elif="line.display_type in ('line_section', 'line_subsection')">
              <td colspan="99"><span t-field="line.name"/></td>
            </t>
          </tr>
        </t>
      </tbody>
    </table>
  </div>
</t></t></template></odoo>"#;

/// A `key=value` connection string's fields, as `data.saveConnection` takes them.
fn saved(conninfo: &str) -> Json {
    let mut out = json!({"name": "odoo", "kind": "postgres", "user": "rc_odoo_reader", "confirm": true});
    for pair in conninfo.split_whitespace() {
        let Some((key, value)) = pair.split_once('=') else { continue };
        match key {
            "host" | "hostaddr" => out["host"] = json!(value),
            "port" => out["port"] = json!(value.parse::<u16>().unwrap()),
            "dbname" => out["database"] = json!(value),
            "sslmode" => out["sslmode"] = json!(value),
            _ => {}
        }
    }
    out
}

#[test]
fn an_imported_template_reads_an_odoo_database_through_its_proposed_queries() {
    let Some(conninfo) = conninfo() else { return };
    let (prepared, tls) = reportcraft_postgres::client_parts(&conninfo).unwrap();
    let mut admin = match tls {
        Some(tls) => Client::connect(&prepared, tls).unwrap(),
        None => Client::connect(&prepared, NoTls).unwrap(),
    };
    admin.batch_execute(SETUP).unwrap();

    let mut s = Session::with_saved_connections(reportcraft_connections::Connections::in_memory());
    s.execute("data.saveConnection", &saved(&conninfo)).unwrap();
    let imported = s.execute("file.importQweb", &json!({"xml": TEMPLATE, "connection": "odoo", "lang": "fr_FR"})).unwrap();
    assert_eq!(imported["data"]["model"], json!("quote.order"), "the model comes from the report action");
    let query = imported["data"]["query"].as_str().unwrap();
    assert!(query.contains("WHERE d.id IN (:ids)"), "{query}");
    let report = s.execute("report.get", &json!({})).unwrap();
    assert_eq!(report["data"]["connection"], json!("odoo"));
    assert_eq!(report["parameters"][0]["name"], json!("ids"));
    assert_eq!(report["parameters"][0]["required"], json!(true));

    let loaded = s.execute("data.loadReportData", &json!({"params": {"ids": [1, 2]}})).unwrap();
    assert_eq!(loaded["rows"], json!(9), "{loaded}");
    let preview = s.execute("data.preview", &json!({"limit": 20})).unwrap();
    let names: Vec<&str> = preview["columns"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    let rows = preview["rows"].as_array().unwrap();
    let column = |name: &str| -> Vec<Json> {
        let i = names.iter().position(|n| *n == name).unwrap_or_else(|| panic!("no column {name} in {names:?}"));
        rows.iter().map(|r| r[i].clone()).collect()
    };
    let text = |name: &str| -> Vec<String> { column(name).iter().map(|v| v.as_str().unwrap_or("").to_owned()).collect() };
    let int = |name: &str| -> Vec<i64> { column(name).iter().map(|v| v.as_i64().unwrap()).collect() };
    // In the documents' order and each document's sequence, not by id.
    assert_eq!(text("line.name"), ["Camp", "Tent line", "Cooking", "Stove line", "Extras", "Extra line", "Loose line", "Later", "Late line"]);
    assert_eq!(int("line.section_number"), [1, 1, 1, 1, 2, 2, 0, 1, 1]);
    assert_eq!(int("line.subsection_number"), [0, 0, 1, 1, 0, 0, 0, 0, 0]);
    assert_eq!(text("o.partner_id")[0], "Partner One", "a record alone shows its name");
    // A related field, translated, with English when the language is missing.
    assert_eq!(text("line.product_id.name")[1..4], ["Tente".to_owned(), String::new(), "Stove".to_owned()]);
    // Records joined in their sequence, each in the language or in English.
    assert_eq!(text("line.tax_ids.name")[1], "Eco, TVA 20 %");
    assert_eq!(text("line.tax_ids.name")[3], "TVA 20 %");

    // The report renders from those rows.
    let rendered = s.execute("report.render", &json!({"params": {"ids": [1, 2]}})).unwrap();
    assert!(rendered.to_string().contains("Tente"), "{rendered}");
}
