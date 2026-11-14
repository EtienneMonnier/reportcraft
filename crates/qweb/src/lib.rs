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
//! apart. Each document starts a new page.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod expr;
mod flow;

use reportcraft_core::{Column, Error, FieldType, Result, RowSet, Value};
use reportcraft_model::{Diagnostic, Group, Report, Severity};
use roxmltree::{Document, Node, ParsingOptions};

use crate::flow::{Builder, Field, Scope};

/// Largest template file read.
pub const MAX_XML_BYTES: usize = 4 << 20;
/// Most XML nodes read (elements, text, attributes).
const MAX_NODES: u32 = 200_000;
/// Documents and lines per document in the sample data.
const SAMPLE_DOCS: usize = 2;
const SAMPLE_LINES: usize = 3;

/// A converted template.
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub report: Report,
    /// Rows shaped like the data the report reads, with made-up values, to preview it.
    pub sample: RowSet,
    /// What wasn't converted, by template line (`line 12`).
    pub diagnostics: Vec<Diagnostic>,
    /// Every template in the file, by name; the one converted is `report.title`.
    pub templates: Vec<String>,
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
    Ok(Import { report, sample, diagnostics, templates: names })
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
                b.fields.insert(0, Field { name: key.clone(), field_type: FieldType::Integer, per_line: false });
            }
            report.groups.push(Group { field: key, keep_together: false, repeat_header: false, page_break_before: true, header, footer });
        }
        None => {
            report.report_header = header;
            report.report_footer = footer;
        }
    }
    report
}

/// Made-up rows: two documents of three lines (or one row per document without a line loop),
/// numbers for fields whose names say amount or quantity, text naming the field otherwise.
fn sample(fields: &[Field]) -> RowSet {
    let columns: Vec<Column> = fields.iter().map(|f| Column { name: f.name.clone(), field_type: f.field_type }).collect();
    let lines = if fields.iter().any(|f| f.per_line) { SAMPLE_LINES } else { 1 };
    let mut rows = Vec::new();
    for d in 1..=SAMPLE_DOCS {
        for l in 1..=lines {
            rows.push(fields.iter().map(|f| sample_value(f, d, l)).collect());
        }
    }
    RowSet { columns, rows }
}

fn sample_value(f: &Field, doc: usize, line: usize) -> Value {
    let n = if f.per_line { line } else { doc };
    let leaf = f.name.rsplit('.').next().unwrap_or(&f.name);
    let picture = ["image", "logo", "photo", "picture", "signature"].iter().any(|w| leaf.to_ascii_lowercase().contains(w));
    match f.field_type {
        _ if picture => Value::Null,
        FieldType::Integer => Value::Int(i64::try_from(n).unwrap_or(1)),
        FieldType::Number => {
            let base = if f.per_line { 12.5 } else { 125.0 };
            Value::Number(base * n as f64)
        }
        FieldType::Bool => Value::Bool(n % 2 == 1),
        FieldType::Text => Value::Text(format!("{} {n}", label(text_leaf(&f.name)))),
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
