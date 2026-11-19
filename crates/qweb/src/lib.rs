//! ReportCraft QWeb import (L2): an Odoo QWeb report template read into a ReportCraft report,
//! with sample rows shaped like the data it expects, so it can be previewed and edited.
//!
//! Only the template's *format* is read, as Odoo documents it (the `t-` directives over HTML); no
//! Odoo code is used or copied (Odoo is LGPL). The conversion is best effort: what has no
//! ReportCraft meaning (Python expressions, other templates called, `t-set`) is reported as a
//! warning with its line, and the report stays usable.
//!
//! The data a converted report reads has one row per line of each document: columns are named as
//! the template writes the fields (`o.partner_id.name`, `line.quantity`), the document's columns
//! repeated on each of its lines, and `o.id` (the document loop's variable) telling documents
//! apart. Each document starts a new page. Another loop over a document's or a line's records
//! reads a dataset of its own, one row per item, shown by a subreport linked to its document (and
//! line); its sample rows come with the import. Odoo's title lines (sections, subsections) group
//! the lines, numbered by `line.section_number` and `line.subsection_number` (see `sections`).
//!
//! [`odoo::propose`] writes the PostgreSQL queries that fill those columns from an Odoo
//! database, from the database's description of its fields.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod expr;
mod flow;
mod letterhead;
pub mod odoo;
mod sections;

use reportcraft_core::{Column, Error, FieldType, Result, RowSet, Value};
use reportcraft_model::{Diagnostic, Group, Report, Severity};
use roxmltree::{Document, Node, ParsingOptions};

use crate::flow::{Builder, Dataset, Field, Level, Scope};

/// Largest template file read.
pub const MAX_XML_BYTES: usize = 4 << 20;
/// Most XML nodes read (elements, text, attributes).
const MAX_NODES: u32 = 200_000;
/// Documents and lines per document in the sample data.
const SAMPLE_DOCS: usize = 2;
const SAMPLE_LINES: usize = 3;
/// Items per document (or line) in a nested loop's sample data.
const SAMPLE_ITEMS: usize = 2;

/// A converted template.
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub report: Report,
    /// Rows shaped like the data the report reads, with made-up values, to preview it.
    pub sample: RowSet,
    /// The datasets of the nested loops (subreports), by name, with made-up rows.
    pub datasets: Vec<(String, RowSet)>,
    /// What wasn't converted, by template line (`line 12`).
    pub diagnostics: Vec<Diagnostic>,
    /// Every template in the file, by name; the one converted is `report.title`.
    pub templates: Vec<String>,
    /// The loops and columns of the report's data, to write its query (`odoo::propose`).
    pub shape: odoo::Shape,
}

/// Converts the template named `template` (its `id` or `t-name`, with or without the module
/// prefix), or the one that is the whole report (see `choose`).
pub fn import(xml: &str, template: Option<&str>) -> Result<Import> {
    if xml.len() > MAX_XML_BYTES {
        return Err(Error::TooLarge(format!("a QWeb file is read up to {} MB", MAX_XML_BYTES >> 20)));
    }
    let options = ParsingOptions { allow_dtd: false, nodes_limit: MAX_NODES, ..ParsingOptions::default() };
    let doc = Document::parse_with_options(xml, options).map_err(|e| Error::Invalid(format!("QWeb file: {e}")))?;
    let templates: Vec<(String, Node<'_, '_>)> = doc.descendants().filter_map(|n| name_of(n).map(|name| (name.to_owned(), n))).collect();
    let names: Vec<String> = templates.iter().map(|(n, _)| n.clone()).collect();
    let (title, mut builder) = match template {
        Some(wanted) => {
            let (title, root) = templates
                .iter()
                .find(|(n, _)| n == wanted || n.rsplit('.').next() == Some(wanted) || wanted.rsplit('.').next() == Some(n.as_str()))
                .ok_or_else(|| Error::NotFound(format!("no template `{wanted}` (templates: {})", names.join(", "))))?;
            (title.as_str(), convert(title, *root, &templates))
        }
        None => choose(&templates)?,
    };
    let mut diagnostics: Vec<Diagnostic> = builder
        .findings
        .iter()
        .map(|f| Diagnostic { severity: f.severity, location: format!("line {}", f.line), message: f.message.clone() })
        .collect();
    diagnostics.dedup();
    if templates.len() > 1 && template.is_none() {
        diagnostics.insert(
            0,
            Diagnostic {
                severity: Severity::Warning,
                location: "file".into(),
                message: format!("the file holds {} templates; `{title}` was read (choose another by name)", templates.len()),
            },
        );
    }
    let report = assemble(title, &mut builder);
    let sample = sample(&builder.fields);
    let datasets = builder.datasets.iter().map(|d| (d.name.clone(), nested_sample(d))).collect();
    let shape = odoo::Shape {
        doc_var: builder.doc_loop.clone(),
        lines: builder.lines.clone(),
        columns: builder.fields.iter().map(|f| f.name.clone()).collect(),
        joined: builder.joined.clone(),
        datasets: builder
            .datasets
            .iter()
            .map(|d| odoo::NestedShape {
                name: d.name.clone(),
                var: d.var.clone(),
                source: d.source.clone(),
                columns: d.fields.iter().map(|f| f.name.clone()).collect(),
                under_line: d.under_line,
            })
            .collect(),
        model: printed_model(&doc, title),
    };
    Ok(Import { report, sample, datasets, diagnostics, templates: names, shape })
}

/// Reads one template into bands.
fn convert<'a, 'i>(title: &str, root: Node<'a, 'i>, library: &[(String, Node<'a, 'i>)]) -> Builder<'a, 'i> {
    let mut builder = Builder::new(library.to_vec());
    builder.enter(title);
    let scope = Scope::default();
    for child in root.children() {
        builder.walk(child, &scope, 0);
    }
    builder.flush(&scope);
    if builder.doc_loop.is_none() && !builder.line_loop {
        builder.warn("no `t-foreach=\"docs\"` loop: the report shows one record");
    }
    builder
}

/// With no template named: the last one that loops over `docs` and shows something (in Odoo's
/// files the document template comes first and the wrapper that loops over the documents and
/// calls it after), else the first that loops over `docs`, else the last template.
fn choose<'a, 'i, 't>(templates: &'t [(String, Node<'a, 'i>)]) -> Result<(&'t str, Builder<'a, 'i>)> {
    let loops = |n: &Node<'_, '_>| n.descendants().any(|d| d.attribute("t-foreach").is_some_and(|e| e.trim() == "docs"));
    let mut fallback = None;
    for (name, root) in templates.iter().rev().filter(|(_, n)| loops(n)) {
        let built = convert(name, *root, templates);
        if built.has_content() {
            return Ok((name.as_str(), built));
        }
        fallback.get_or_insert((name.as_str(), built));
    }
    if let Some(found) = fallback {
        return Ok(found);
    }
    let (name, root) = templates.last().ok_or_else(|| Error::Invalid("QWeb file: no `<template>` and no element with `t-name`".into()))?;
    Ok((name.as_str(), convert(name, *root, templates)))
}

/// The model the file's report action prints: an `ir.actions.report` record (or an older
/// `<report>` element) whose report name is the template, else the only one in the file.
fn printed_model(doc: &Document<'_>, template: &str) -> Option<String> {
    let short = |n: &str| n.rsplit('.').next().unwrap_or(n).to_owned();
    let mut actions: Vec<(String, String)> = Vec::new();
    for n in doc.descendants().filter(Node::is_element) {
        let field = |name: &str| {
            n.children()
                .find(|c| c.tag_name().name() == "field" && c.attribute("name") == Some(name))
                .and_then(|c| c.text())
                .map(|t| t.trim().to_owned())
        };
        match n.tag_name().name() {
            "record" if n.attribute("model") == Some("ir.actions.report") => {
                if let (Some(model), Some(name)) = (field("model"), field("report_name")) {
                    actions.push((model, name));
                }
            }
            "report" => {
                if let (Some(model), Some(name)) = (n.attribute("model"), n.attribute("name")) {
                    actions.push((model.to_owned(), name.to_owned()));
                }
            }
            _ => {}
        }
    }
    let named = actions.iter().find(|(_, name)| short(name) == short(template)).map(|(m, _)| m.clone());
    named.or_else(|| match actions.as_slice() {
        [(model, _)] => Some(model.clone()),
        _ => None,
    })
}

/// A template's name: `<template id>` or `t-name`.
fn name_of<'a>(n: Node<'a, '_>) -> Option<&'a str> {
    if !n.is_element() {
        return None;
    }
    if n.tag_name().name() == "template" {
        return n.attribute("id").or_else(|| n.attribute("t-name"));
    }
    n.attribute("t-name")
}

/// The bands as a report: a group per document when the template loops over `docs`.
fn assemble(title: &str, b: &mut Builder) -> Report {
    let mut report = Report::new(title);
    let header = std::mem::take(&mut b.header);
    let footer = std::mem::take(&mut b.footer);
    report.detail = std::mem::take(&mut b.detail);
    match b.doc_loop.clone() {
        Some(var) => {
            let key = format!("{var}.id");
            if !b.fields.iter().any(|f| f.name.eq_ignore_ascii_case(&key)) {
                b.fields.insert(0, Field { name: key.clone(), field_type: FieldType::Integer, level: Level::Doc, example: None });
            }
            report.groups.push(Group {
                field: key,
                keep_together: false,
                repeat_header: false,
                page_break_before: true,
                header,
                footer,
                restart_page_numbers: true,
            });
        }
        None => {
            report.report_header = header;
            report.report_footer = footer;
        }
    }
    sections::apply(&mut report, &mut b.fields);
    shrink(&mut report);
    if let Some((layout, doc)) = b.letterhead.clone() {
        letterhead::apply(&mut report, layout, &doc, &mut b.fields);
    }
    report
}

/// The body's bands, and their subreports', give back the room of what Odoo wouldn't print (a
/// hidden branch, an empty field), as HTML collapses an empty block.
fn shrink(report: &mut Report) {
    let groups = report.groups.iter_mut().flat_map(|g| [&mut g.header, &mut g.footer]);
    let bands = [&mut report.report_header, &mut report.detail, &mut report.report_footer].into_iter().chain(groups);
    for band in bands {
        band.can_shrink = true;
        for el in &mut band.elements {
            if let reportcraft_model::Content::Subreport { report: child, .. } = &mut el.content {
                shrink(child);
            }
        }
    }
}

/// Made-up rows: two documents of three lines (or one row per document without a line loop),
/// numbers for fields whose names say amount or quantity, text naming the field otherwise.
fn sample(fields: &[Field]) -> RowSet {
    let columns: Vec<Column> = fields.iter().map(|f| Column { name: f.name.clone(), field_type: f.field_type }).collect();
    let lines = if fields.iter().any(|f| f.level == Level::Line) { SAMPLE_LINES } else { 1 };
    // Lines grouped by section: the first line of each document is a section's title.
    let sections = fields.iter().any(|f| f.name.ends_with(".section_number"));
    let mut rows = Vec::new();
    for d in 1..=SAMPLE_DOCS {
        for l in 1..=lines {
            rows.push(fields.iter().map(|f| if sections { section_value(f, l) } else { None }.unwrap_or_else(|| sample_value(f, d, l, 1))).collect());
        }
    }
    RowSet { columns, rows }
}

/// A line's value in sample rows grouped by section, where it differs from the others: line 1
/// is the title (no amounts), the others products, all in section 1.
fn section_value(f: &Field, line: usize) -> Option<Value> {
    if f.level != Level::Line {
        return None;
    }
    let title = line == 1;
    if f.name.ends_with(".display_type") {
        return Some(Value::Text(if title { "line_section" } else { "product" }.to_owned()));
    }
    if f.name.ends_with(".section_number") {
        return Some(Value::Int(1));
    }
    if f.name.ends_with(".subsection_number") {
        return Some(Value::Int(0));
    }
    (title && f.field_type == FieldType::Number).then_some(Value::Null)
}

/// A nested loop's made-up rows: two items for each document of the sample data, or for each of
/// its lines, with the same document and line values as the main rows so the links match.
fn nested_sample(d: &Dataset) -> RowSet {
    let columns: Vec<Column> = d.fields.iter().map(|f| Column { name: f.name.clone(), field_type: f.field_type }).collect();
    let lines = if d.under_line { SAMPLE_LINES } else { 1 };
    let mut rows = Vec::new();
    for doc in 1..=SAMPLE_DOCS {
        for line in 1..=lines {
            for item in 1..=SAMPLE_ITEMS {
                rows.push(d.fields.iter().map(|f| sample_value(f, doc, line, item)).collect());
            }
        }
    }
    RowSet { columns, rows }
}

fn sample_value(f: &Field, doc: usize, line: usize, item: usize) -> Value {
    let n = match f.level {
        Level::Doc => doc,
        Level::Line => line,
        Level::Item => item,
    };
    let leaf = f.name.rsplit('.').next().unwrap_or(&f.name);
    let picture = ["image", "logo", "photo", "picture", "signature"].iter().any(|w| leaf.to_ascii_lowercase().contains(w));
    match f.field_type {
        _ if picture => Value::Null,
        FieldType::Integer => Value::Int(i64::try_from(n).unwrap_or(1)),
        FieldType::Number => {
            let base = match f.level {
                Level::Doc => 125.0,
                Level::Line => 12.5,
                Level::Item => 10.0,
            };
            Value::Number(base * n as f64)
        }
        // Flags that hide content (`collapse_prices`, `hide_…`) are off, others on.
        FieldType::Bool => Value::Bool(!["collapse", "hide"].iter().any(|w| leaf.starts_with(w))),
        FieldType::Text => match &f.example {
            Some(example) => Value::Text(example.clone()),
            None => Value::Text(format!("{} {n}", label(text_leaf(&f.name)))),
        },
    }
}

/// The part of a path that names what a text shows: `o.partner_id.name` is a partner.
fn text_leaf(path: &str) -> &str {
    let parts: Vec<&str> = path.split('.').skip(1).collect();
    match parts.as_slice() {
        [.., owner, "name" | "display_name"] => owner,
        [.., last] => last,
        [] => path,
    }
}

/// `partner_id` → `Partner`, `date_invoice` → `Date invoice`.
fn label(leaf: &str) -> String {
    let words = leaf.strip_suffix("_id").or_else(|| leaf.strip_suffix("_ids")).unwrap_or(leaf).replace('_', " ");
    let mut chars = words.trim().chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => "Value".to_owned(),
    }
}
