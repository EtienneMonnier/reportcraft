//! The SQL that fills a converted template from an Odoo database: each column the report reads
//! (`o.partner_id.name`, `line.product_id.name`) followed through Odoo's fields — many2one fields
//! as joins, translated fields in the chosen language, records joined into a text with
//! `string_agg`, pictures as their attachment's file — into one PostgreSQL query per dataset.
//!
//! What the fields are comes from the database's own description of them (`ir_model_fields`,
//! read with [`metadata_query`]), never from Odoo's code. Reading it is the caller's work: a
//! [`Proposal`] lists the models it still needs (`missing`), the caller reads them into the
//! [`OdooSchema`] and asks again, until nothing is missing. Each round follows the paths one
//! join further.

use std::collections::{BTreeMap, BTreeSet};

use reportcraft_core::{Error, Result, RowSet, Value};
use reportcraft_model::{Diagnostic, Severity};

/// What the data of a converted template looks like: the loops and the columns read under them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Shape {
    /// The document loop's variable (`o`).
    pub doc_var: Option<String>,
    /// The line loop's variable and what it loops over (`o.invoice_line_ids`, or a `t-set` name
    /// for Python such as `lines_to_report`).
    pub lines: Option<(String, String)>,
    /// The report's columns, as the template names them.
    pub columns: Vec<String>,
    /// Columns holding records' values joined into a text, with their separator.
    pub joined: Vec<(String, String)>,
    /// The nested loops' datasets.
    pub datasets: Vec<NestedShape>,
    /// The model the file's report action prints (`account.move`), when the file declares one.
    pub model: Option<String>,
}

/// A nested loop's dataset: its name, what it loops over (`o.payment_ids`) with its variable,
/// and its columns.
#[derive(Debug, Clone, PartialEq)]
pub struct NestedShape {
    pub name: String,
    pub var: String,
    pub source: String,
    pub columns: Vec<String>,
    pub under_line: bool,
}

/// One Odoo field, as `ir_model_fields` describes it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OdooField {
    pub ttype: String,
    pub relation: Option<String>,
    pub relation_field: Option<String>,
    pub relation_table: Option<String>,
    pub column1: Option<String>,
    pub column2: Option<String>,
    pub store: bool,
    /// `product_tmpl_id.name` for a field that shows another's value.
    pub related: Option<String>,
    pub translate: bool,
    pub company_dependent: bool,
}

/// The fields of the models read so far.
#[derive(Debug, Clone, Default)]
pub struct OdooSchema {
    models: BTreeMap<String, BTreeMap<String, OdooField>>,
}

/// Most models one proposal reads (a join per segment of every path).
pub const MAX_MODELS: usize = 64;
/// Related fields followed one through another at most this deep.
const MAX_RELATED: usize = 8;

/// The SQL that reads the fields of `models` from `ir_model_fields`. `translate` and
/// `company_dependent` are read through `to_jsonb` because their column changed type (boolean,
/// then text) or doesn't exist in every version. Names that aren't model names are left out.
pub fn metadata_query(models: &[String]) -> String {
    let names: Vec<String> = models.iter().filter(|m| is_model_name(m)).map(|m| literal(m)).collect();
    let list = if names.is_empty() { "NULL".to_owned() } else { names.join(", ") };
    format!(
        "SELECT f.model, f.name, f.ttype, f.relation, f.relation_field, f.relation_table, f.column1, f.column2, f.store, f.related, \
         to_jsonb(f) ->> 'translate' AS translate, to_jsonb(f) ->> 'company_dependent' AS company_dependent \
         FROM ir_model_fields f WHERE f.model IN ({list}) ORDER BY f.model, f.name"
    )
}

fn is_model_name(m: &str) -> bool {
    !m.is_empty() && m.len() <= 128 && m.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_')
}

impl OdooSchema {
    /// Adds the rows of [`metadata_query`] over `models`; each of them counts as read, even with
    /// no rows (a model the database doesn't have).
    pub fn add(&mut self, models: &[String], rows: &RowSet) -> Result<()> {
        let index = |name: &str| rows.columns.iter().position(|c| c.name.eq_ignore_ascii_case(name));
        let need = |name: &str| index(name).ok_or_else(|| Error::Invalid(format!("Odoo fields: no `{name}` column")));
        let (model, name, ttype) = (need("model")?, need("name")?, need("ttype")?);
        let others =
            ["relation", "relation_field", "relation_table", "column1", "column2", "store", "related", "translate", "company_dependent"].map(index);
        for m in models {
            self.models.entry(m.clone()).or_default();
        }
        for row in &rows.rows {
            let text = |i: Option<usize>| match i.and_then(|i| row.get(i)) {
                Some(Value::Text(t)) if !t.is_empty() => Some(t.clone()),
                _ => None,
            };
            let truth = |i: Option<usize>| match i.and_then(|i| row.get(i)) {
                Some(Value::Bool(b)) => *b,
                Some(Value::Text(t)) => !matches!(t.as_str(), "" | "false" | "f" | "0"),
                Some(Value::Int(n)) => *n != 0,
                _ => false,
            };
            let (Some(m), Some(n)) = (text(Some(model)), text(Some(name))) else { continue };
            let [relation, relation_field, relation_table, column1, column2, store, related, translate, company_dependent] = others;
            let field = OdooField {
                ttype: text(Some(ttype)).unwrap_or_default(),
                relation: text(relation),
                relation_field: text(relation_field),
                relation_table: text(relation_table),
                column1: text(column1),
                column2: text(column2),
                store: truth(store),
                related: text(related),
                translate: truth(translate),
                company_dependent: truth(company_dependent),
            };
            self.models.entry(m).or_default().insert(n, field);
        }
        Ok(())
    }

    /// Adds one field by hand (tests, or a description read another way).
    pub fn insert(&mut self, model: &str, name: &str, field: OdooField) {
        self.models.entry(model.to_owned()).or_default().insert(name.to_owned(), field);
    }

    /// Whether the model was read and has fields.
    pub fn has_model(&self, model: &str) -> bool {
        self.models.get(model).is_some_and(|f| !f.is_empty())
    }

    fn lookup(&self, model: &str, name: &str) -> Lookup<'_> {
        match self.models.get(model) {
            None => Lookup::Pending,
            Some(fields) => fields.get(name).map_or(Lookup::Unknown, Lookup::Found),
        }
    }
}

enum Lookup<'s> {
    /// The model hasn't been read yet.
    Pending,
    Unknown,
    Found(&'s OdooField),
}

/// What the query is for.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryOptions {
    /// The model of the documents (`account.move`); default: the one the file's report action
    /// names.
    pub model: Option<String>,
    /// The field the lines come from (`invoice_line_ids`), when the template loops over Python.
    pub lines: Option<String>,
    /// The language translated fields are read in (`fr_FR`), falling back to `en_US`.
    pub lang: String,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self { model: None, lines: None, lang: "en_US".into() }
    }
}

/// The proposed queries, what they couldn't read, and the models still to read.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Proposal {
    pub model: String,
    pub query: String,
    /// Each nested loop's dataset and its query.
    pub datasets: Vec<(String, String)>,
    pub diagnostics: Vec<Diagnostic>,
    /// Models whose fields are needed and not read yet: read them and propose again.
    pub missing: Vec<String>,
}

/// The report parameter the queries filter the documents with (`d.id IN (:ids)`).
pub const IDS_PARAMETER: &str = "ids";

/// Line fields of a few documents, for templates that loop over Python (`lines_to_report`):
/// the documented field that lists a document's lines.
const KNOWN_LINES: [(&str, &str); 4] =
    [("account.move", "invoice_line_ids"), ("sale.order", "order_line"), ("purchase.order", "order_line"), ("stock.picking", "move_ids")];

/// Line fields that show only some of the records their inverse field links (Odoo filters them
/// with a domain the database doesn't describe): an invoice's lines leave out its tax and
/// payment term lines.
const KNOWN_DOMAINS: [(&str, &str, &str, &str); 1] =
    [("account.move", "invoice_line_ids", "display_type", "'product', 'line_section', 'line_subsection', 'line_note'")];

/// Fields Odoo computes from another record's, without saying so in the database: a company's
/// address is its partner's.
const KNOWN_RELATED: [(&str, &[&str], &str); 1] = [("res.company", &["street", "street2", "zip", "city", "state_id", "country_id"], "partner_id")];

/// Values an invoice computes in Python (its tax totals, a dictionary) that the query reads from
/// stored fields, by their path under the document: `{d}` is the document's alias.
const KNOWN_VALUES: [(&str, &str, &str); 12] = [
    ("account.move", "tax_totals", "TRUE"),
    ("account.move", "tax_totals.total_amount_currency", "{d}.amount_total"),
    ("account.move", "tax_totals.total_amount", "abs({d}.amount_total_signed)"),
    ("account.move", "tax_totals.base_amount_currency", "{d}.amount_untaxed"),
    ("account.move", "tax_totals.base_amount", "abs({d}.amount_untaxed_signed)"),
    ("account.move", "tax_totals.tax_amount_currency", "{d}.amount_tax"),
    ("account.move", "tax_totals.same_tax_base", "TRUE"),
    (
        "account.move",
        "tax_totals.display_in_company_currency",
        "{d}.currency_id IS DISTINCT FROM (SELECT rc.currency_id FROM res_company rc WHERE rc.id = {d}.company_id)",
    ),
    ("account.move", "tax_totals.subtotals.name", "'Untaxed Amount'"),
    ("account.move", "tax_totals.subtotals.base_amount_currency", "{d}.amount_untaxed"),
    ("account.move", "tax_totals.subtotals.base_amount", "abs({d}.amount_untaxed_signed)"),
    (
        "account.move",
        "tax_totals.subtotals.tax_groups",
        "EXISTS (SELECT 1 FROM account_move_line x WHERE x.move_id = {d}.id AND x.display_type = 'tax')",
    ),
];

/// The tax groups of an invoice's totals: its tax lines summed by tax group.
const TAX_GROUPS: &str = "tax_totals.subtotals.tax_groups";

/// Whether a loop goes over a value that holds one item per document, read once with its
/// variable naming that value: an invoice's tax subtotals (one, "Untaxed Amount", as Odoo prints
/// an invoice with no subtotals of its own).
pub(crate) fn single_loop(each: &str) -> bool {
    each.ends_with(".tax_totals.subtotals")
}

/// A computed label (`display_name`, `tax_label`): Python builds it, mostly from the record's name.
fn is_label(field: &str) -> bool {
    field == "display_name" || field == "label" || field.ends_with("_label")
}

const SECTION: &str = "section_number";
const SUBSECTION: &str = "subsection_number";

/// Proposes the queries of a converted template over `schema`. Fails only when there is nothing
/// to write a query for (no document loop, no model).
pub fn propose(shape: &Shape, options: &QueryOptions, schema: &OdooSchema) -> Result<Proposal> {
    let doc_var = shape
        .doc_var
        .clone()
        .ok_or_else(|| Error::Invalid("the template has no `t-foreach=\"docs\"` loop: there are no documents to query".into()))?;
    let model = options.model.clone().or_else(|| shape.model.clone()).ok_or_else(|| {
        Error::Invalid("name the Odoo model the template prints (`model`, such as `account.move`): the file has no report action for it".into())
    })?;
    if !is_model_name(&model) {
        return Err(Error::Invalid(format!("`{model}` isn't an Odoo model name")));
    }
    let lang = if options.lang.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '@') && !options.lang.is_empty() {
        options.lang.clone()
    } else {
        return Err(Error::Invalid(format!("`{}` isn't a language code (`fr_FR`)", options.lang)));
    };
    let mut q = Query { schema, lang, joins: Vec::new(), next: 0, missing: BTreeSet::new(), diagnostics: Vec::new() };
    let mut proposal = Proposal { model: model.clone(), ..Proposal::default() };
    if matches!(schema.models.get(&model), Some(f) if f.is_empty()) {
        return Err(Error::NotFound(format!("model `{model}` isn't in this database (is its module installed?)")));
    }
    let doc = Root { var: doc_var, alias: "d".into(), model: model.clone() };
    let lines = q.lines_root(shape, options, &doc);
    proposal.query = q.main(shape, &doc, lines.as_ref());
    for nested in &shape.datasets {
        q.joins.clear();
        let sql = q.nested(shape, nested, &doc, lines.as_ref());
        proposal.datasets.push((nested.name.clone(), sql));
    }
    proposal.diagnostics = q.diagnostics;
    proposal.diagnostics.dedup();
    proposal.missing = q.missing.into_iter().take(MAX_MODELS).collect();
    Ok(proposal)
}

/// A loop's variable, the alias of its table in the query and its model.
#[derive(Debug, Clone)]
struct Root {
    var: String,
    alias: String,
    model: String,
}

/// The line loop: its root, the join that brings its rows, and the order of its rows.
struct Lines {
    root: Root,
    join: Linked,
    order: String,
}

/// How records are reached from their parent: the tables (one, or a relation table and the
/// records') and the condition on the parent.
struct Linked {
    tables: String,
    on: String,
}

impl Linked {
    /// `"account_move_line" l ON l."move_id" = d.id`, without the kind of join.
    fn join(&self) -> String {
        if self.tables.contains(" JOIN ") { format!("({}) ON {}", self.tables, self.on) } else { format!("{} ON {}", self.tables, self.on) }
    }
}

/// A value read for a column: SQL, or why it can't be (the column reads NULL).
enum Read {
    Sql(String),
    Unread(String),
    /// A model on the way hasn't been read yet.
    Pending,
}

struct Join {
    key: (String, String),
    alias: String,
    sql: String,
}

struct Query<'s> {
    schema: &'s OdooSchema,
    lang: String,
    joins: Vec<Join>,
    next: usize,
    missing: BTreeSet<String>,
    diagnostics: Vec<Diagnostic>,
}

impl<'s> Query<'s> {
    fn note(&mut self, severity: Severity, location: &str, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic { severity, location: format!("query: {location}"), message: message.into() });
    }

    fn alias(&mut self) -> String {
        self.next += 1;
        format!("t{}", self.next)
    }

    fn lookup(&mut self, model: &str, name: &str) -> Lookup<'s> {
        let schema = self.schema;
        let found = schema.lookup(model, name);
        if matches!(found, Lookup::Pending) {
            self.missing.insert(model.to_owned());
        }
        found
    }

    /// The line loop's root: the field its source names under the document, else the one given,
    /// else the documented one of the model.
    fn lines_root(&mut self, shape: &Shape, options: &QueryOptions, doc: &Root) -> Option<Lines> {
        let (var, source) = shape.lines.clone()?;
        let from_doc = source.strip_prefix(&doc.var).and_then(|r| r.strip_prefix('.')).filter(|r| !r.is_empty() && is_path(r)).map(str::to_owned);
        let field = match (from_doc, &options.lines) {
            (_, Some(given)) => given.clone(),
            (Some(path), None) => path,
            (None, None) => match KNOWN_LINES.iter().find(|(m, _)| *m == doc.model) {
                Some((_, f)) => {
                    self.note(
                        Severity::Warning,
                        &var,
                        format!("the lines come from Python (`{source}`): read from `{f}` (name another field with `lines`)"),
                    );
                    (*f).to_owned()
                }
                None => {
                    self.note(Severity::Error, &var, format!("the lines come from Python (`{source}`): name the field they come from with `lines`"));
                    return None;
                }
            },
        };
        let path: Vec<&str> = field.split('.').collect();
        match self.records(&doc.alias, &doc.model, &path, "l", &var) {
            Some((join, model)) => {
                let order = self.order("l", &model);
                Some(Lines { root: Root { var, alias: "l".into(), model }, join, order })
            }
            None => None,
        }
    }

    /// The join that brings the records of the x2many field at the end of `path` (many2one
    /// fields before it joined as usual), as `alias`, with their model. `None` when the path
    /// doesn't lead to records (said in a diagnostic) or isn't read yet.
    fn records(&mut self, from: &str, model: &str, path: &[&str], alias: &str, location: &str) -> Option<(Linked, String)> {
        let (mut at, mut model) = (from.to_owned(), model.to_owned());
        for (i, seg) in path.iter().enumerate() {
            let last = i + 1 == path.len();
            let field = match self.lookup(&model, seg) {
                Lookup::Pending => return None,
                Lookup::Unknown => {
                    self.note(Severity::Error, location, format!("`{model}` has no field `{seg}`"));
                    return None;
                }
                Lookup::Found(f) => f.clone(),
            };
            let relation = field.relation.clone().unwrap_or_default();
            match field.ttype.as_str() {
                "many2one" if !last => {
                    at = self.many2one(&at, seg, &relation);
                    model = relation;
                }
                "one2many" | "many2many" if last => {
                    let table = table(&relation);
                    let mut linked = if field.ttype == "one2many" {
                        let inverse = field.relation_field.clone().unwrap_or_default();
                        Linked { tables: format!("{table} {alias}"), on: format!("{alias}.{} = {at}.id", ident(&inverse)) }
                    } else {
                        let (Some(rel), Some(c1), Some(c2)) = (&field.relation_table, &field.column1, &field.column2) else {
                            self.note(Severity::Error, location, format!("`{model}.{seg}` has no relation table"));
                            return None;
                        };
                        let r = format!("{alias}_rel");
                        Linked {
                            tables: format!("{} {r} JOIN {table} {alias} ON {alias}.id = {r}.{}", ident(rel), ident(c2)),
                            on: format!("{r}.{} = {at}.id", ident(c1)),
                        }
                    };
                    let filter = KNOWN_DOMAINS
                        .iter()
                        .find(|(m, f, _, _)| *m == model && f == seg)
                        .map(|(_, _, field, values)| format!(" AND {alias}.{} IN ({values})", ident(field)))
                        .unwrap_or_default();
                    if !filter.is_empty() {
                        self.note(
                            Severity::Warning,
                            location,
                            format!("`{model}.{seg}` keeps the records Odoo shows there (`{}`)", filter.trim_start_matches(" AND ")),
                        );
                    }
                    linked.on.push_str(&filter);
                    return Some((linked, relation));
                }
                _ if !field.store && field.related.is_some() => {
                    let related = field.related.clone().unwrap_or_default();
                    let mut full: Vec<&str> = related.split('.').collect();
                    full.extend_from_slice(&path[i + 1..]);
                    return self.records(&at, &model, &full, alias, location);
                }
                other => {
                    self.note(Severity::Error, location, format!("`{model}.{seg}` is a {other} field, not records to loop over"));
                    return None;
                }
            }
        }
        None
    }

    /// The alias of the record a many2one field points to, joined once per field.
    fn many2one(&mut self, from: &str, field: &str, relation: &str) -> String {
        let key = (from.to_owned(), field.to_owned());
        if let Some(j) = self.joins.iter().find(|j| j.key == key) {
            return j.alias.clone();
        }
        let alias = self.alias();
        let sql = format!("LEFT JOIN {} {alias} ON {alias}.id = {from}.{}", table(relation), ident(field));
        self.joins.push(Join { key, alias: alias.clone(), sql });
        alias
    }

    /// The order of a model's records: by `sequence` when it has one, then by id.
    fn order(&mut self, alias: &str, model: &str) -> String {
        match self.lookup(model, "sequence") {
            Lookup::Found(f) if f.store => format!("{alias}.\"sequence\", {alias}.id"),
            _ => format!("{alias}.id"),
        }
    }

    /// A column's SQL, with `NULL` and a diagnostic when it can't be read.
    fn column(&mut self, name: &str, roots: &[&Root], shape: &Shape) -> String {
        let mut parts = name.split('.');
        let var = parts.next().unwrap_or_default();
        let path: Vec<&str> = parts.collect();
        let Some(root) = roots.iter().find(|r| r.var == var) else {
            self.note(Severity::Warning, name, format!("`{var}` isn't a loop over records: the column reads NULL"));
            return "NULL".into();
        };
        let read = match shape.joined.iter().find(|(c, _)| c.eq_ignore_ascii_case(name)) {
            Some((_, separator)) => self.joined(&root.alias, &root.model, &path, separator, 0),
            None => self.value(&root.alias, &root.model, &path, 0),
        };
        match read {
            Read::Sql(sql) => sql,
            Read::Pending => "NULL".into(),
            Read::Unread(why) => {
                self.note(Severity::Warning, name, format!("{why}: the column reads NULL"));
                "NULL".into()
            }
        }
    }

    /// The value at the end of `path` from the record `alias` of `model`.
    fn value(&mut self, alias: &str, model: &str, path: &[&str], depth: usize) -> Read {
        let Some((seg, rest)) = path.split_first() else { return Read::Unread("no field named".into()) };
        if depth > MAX_RELATED {
            return Read::Unread("related fields lead through each other too deep".into());
        }
        if *seg == "id" && rest.is_empty() {
            return Read::Sql(format!("{alias}.id"));
        }
        let full = path.join(".");
        if let Some((_, _, sql)) = KNOWN_VALUES.iter().find(|(m, p, _)| *m == model && *p == full) {
            self.note(
                Severity::Warning,
                &format!("{model}.{full}"),
                format!("`{model}.{full}` is computed in Python: the query reads `{}`", sql.replace("{d}.", "")),
            );
            return Read::Sql(sql.replace("{d}", alias));
        }
        let field = match self.lookup(model, seg) {
            Lookup::Pending => return Read::Pending,
            Lookup::Unknown if *seg == "display_name" && rest.is_empty() => return self.value(alias, model, &["name"], depth + 1),
            Lookup::Unknown => return Read::Unread(format!("`{model}` has no field `{seg}`")),
            Lookup::Found(f) => f.clone(),
        };
        if !field.store {
            if let Some(related) = &field.related {
                let mut full: Vec<&str> = related.split('.').collect();
                full.extend_from_slice(rest);
                return self.value(alias, model, &full, depth + 1);
            }
            if let Some((_, _, through)) = KNOWN_RELATED.iter().find(|(m, fields, _)| *m == model && fields.contains(seg)) {
                let mut full = vec![*through, *seg];
                full.extend_from_slice(rest);
                return self.value(alias, model, &full, depth + 1);
            }
            // Odoo computes it in Python; a record's label falls back on its name.
            if rest.is_empty()
                && matches!(field.ttype.as_str(), "char" | "text")
                && is_label(seg)
                && let Lookup::Found(name) = self.lookup(model, "name")
                && name.store
            {
                self.note(
                    Severity::Warning,
                    &format!("{model}.{seg}"),
                    format!("`{model}.{seg}` is computed in Python: the query reads the record's `name`"),
                );
                return self.value(alias, model, &["name"], depth + 1);
            }
            return Read::Unread(format!("`{model}.{seg}` is computed in Python, not stored"));
        }
        let column = format!("{alias}.{}", ident(seg));
        match field.ttype.as_str() {
            "many2one" => {
                let relation = field.relation.clone().unwrap_or_default();
                // A record shown by itself (`t-field="o.partner_id"`) shows its name, if it has one.
                let rest: &[&str] = match (rest.is_empty(), self.lookup(&relation, "name")) {
                    (false, _) => rest,
                    (true, Lookup::Pending) => return Read::Pending,
                    (true, Lookup::Unknown) => return Read::Sql(column),
                    (true, Lookup::Found(_)) => &["name"],
                };
                let target = self.many2one(alias, seg, &relation);
                self.value(&target, &relation, rest, depth + 1)
            }
            "one2many" | "many2many" => Read::Unread(format!("`{model}.{seg}` holds several records (a loop, or a join into a text, reads them)")),
            _ if !rest.is_empty() => Read::Unread(format!("`{model}.{seg}` isn't a record")),
            "binary" => Read::Sql(format!(
                "(SELECT a.store_fname FROM ir_attachment a WHERE a.res_model = {} AND a.res_field = {} AND a.res_id = {alias}.id ORDER BY a.id DESC LIMIT 1)",
                literal(model),
                literal(seg)
            )),
            _ if field.company_dependent => Read::Unread(format!("`{model}.{seg}` depends on the company (not read yet)")),
            _ if field.translate => Read::Sql(self.translated(&column)),
            _ => Read::Sql(column),
        }
    }

    fn translated(&self, column: &str) -> String {
        if self.lang == "en_US" {
            format!("{column} ->> 'en_US'")
        } else {
            format!("coalesce({column} ->> {}, {column} ->> 'en_US')", literal(&self.lang))
        }
    }

    /// Records' values joined into a text: many2one fields up to the records joined as usual,
    /// then a subquery that joins the values with `separator`.
    fn joined(&mut self, alias: &str, model: &str, path: &[&str], separator: &str, depth: usize) -> Read {
        let Some((seg, rest)) = path.split_first() else { return Read::Unread("no field named".into()) };
        if depth > MAX_RELATED {
            return Read::Unread("related fields lead through each other too deep".into());
        }
        let field = match self.lookup(model, seg) {
            Lookup::Pending => return Read::Pending,
            Lookup::Unknown => return Read::Unread(format!("`{model}` has no field `{seg}`")),
            Lookup::Found(f) => f.clone(),
        };
        let relation = field.relation.clone().unwrap_or_default();
        match field.ttype.as_str() {
            _ if !field.store && field.related.is_some() => {
                let related = field.related.clone().unwrap_or_default();
                let mut full: Vec<&str> = related.split('.').collect();
                full.extend_from_slice(rest);
                self.joined(alias, model, &full, separator, depth + 1)
            }
            "many2one" => {
                let target = self.many2one(alias, seg, &relation);
                self.joined(&target, &relation, rest, separator, depth + 1)
            }
            "one2many" | "many2many" => {
                // The subquery's own joins, kept apart from the query's.
                let outer = std::mem::take(&mut self.joins);
                let item = self.alias();
                let source = self.records(alias, model, &[seg], &item, &format!("{model}.{seg}"));
                let rest: &[&str] = if rest.is_empty() { &["name"] } else { rest };
                let value = self.value(&item, &relation, rest, depth + 1);
                let order = self.order(&item, &relation);
                let inner = std::mem::replace(&mut self.joins, outer);
                let (source, value) = match (source, value) {
                    (Some((source, _)), Read::Sql(value)) => (source, value),
                    (_, Read::Unread(why)) => return Read::Unread(why),
                    (None, _) => return Read::Unread(format!("`{model}.{seg}` can't be joined")),
                    (_, Read::Pending) => return Read::Pending,
                };
                let joins: String = inner.iter().map(|j| format!(" {}", j.sql)).collect();
                Read::Sql(format!(
                    "(SELECT string_agg(({value})::text, {} ORDER BY {order}) FROM {}{joins} WHERE {})",
                    literal(separator),
                    source.tables,
                    source.on
                ))
            }
            _ => self.value(alias, model, path, depth),
        }
    }

    /// The report's query: one row per line of each document.
    fn main(&mut self, shape: &Shape, doc: &Root, lines: Option<&Lines>) -> String {
        let mut roots = vec![doc];
        if let Some(l) = lines {
            roots.push(&l.root);
        }
        let numbered = |c: &str, which: &str| lines.is_some_and(|l| c.strip_prefix(&l.root.var).and_then(|r| r.strip_prefix('.')) == Some(which));
        let sections = shape.columns.iter().any(|c| numbered(c, SECTION) || numbered(c, SUBSECTION));
        let mut select = Vec::new();
        for c in &shape.columns {
            if sections && (numbered(c, SECTION) || numbered(c, SUBSECTION)) {
                continue;
            }
            let sql = self.column(c, &roots, shape);
            select.push(format!("{sql} AS {}", ident(c)));
        }
        let mut from = format!("FROM {} d", table(&doc.model));
        let mut order = "d.id".to_owned();
        if let Some(l) = lines {
            from.push_str(&format!("\nLEFT JOIN {}", l.join.join()));
            order = format!("d.id, {}", l.order);
        }
        for j in &self.joins {
            from.push_str(&format!("\n{}", j.sql));
        }
        let filter = format!("WHERE d.id IN (:{IDS_PARAMETER})");
        let Some(l) = lines.filter(|_| sections) else {
            return format!("SELECT\n  {}\n{from}\n{filter}\nORDER BY {order}", select.join(",\n  "));
        };
        // Section numbers count the title lines before each line, in the document's order; a
        // subsection's restarts at each section, which needs the first count (a second level).
        let kind = format!("{}.\"display_type\"", l.root.alias);
        select.push(format!("count(*) FILTER (WHERE {kind} = 'line_section') OVER w AS {}", ident(&format!("{}.{SECTION}", l.root.var))));
        select.push(format!("count(*) FILTER (WHERE {kind} = 'line_subsection') OVER w AS rc_subsections"));
        select.push(format!("{kind} AS rc_type"));
        select.push(format!("row_number() OVER (ORDER BY {order}) AS rc_order"));
        let outer: Vec<String> = shape
            .columns
            .iter()
            .map(|c| {
                if numbered(c, SUBSECTION) {
                    format!(
                        "q.rc_subsections - coalesce(max(CASE WHEN q.rc_type = 'line_section' THEN q.rc_subsections END) OVER (PARTITION BY q.{} ORDER BY q.rc_order), 0) AS {}",
                        ident(&format!("{}.id", doc.var)),
                        ident(c)
                    )
                } else {
                    format!("q.{}", ident(c))
                }
            })
            .collect();
        format!(
            "SELECT\n  {}\nFROM (\nSELECT\n  {}\n{from}\n{filter}\nWINDOW w AS (PARTITION BY d.id ORDER BY {})\n) q\nORDER BY q.rc_order",
            outer.join(",\n  "),
            select.join(",\n  "),
            l.order
        )
    }

    /// An invoice's tax groups (`tax_totals['subtotals'][…]['tax_groups']`, computed in Python):
    /// its tax lines summed by tax group, amounts signed as the invoice shows them (a customer
    /// invoice's taxes are credits), the base from the tax lines' own base (company currency).
    fn tax_groups(&mut self, shape: &Shape, nested: &NestedShape, doc: &Root) -> String {
        self.note(
            Severity::Warning,
            &nested.name,
            "an invoice's tax groups are computed in Python: the query sums its tax lines by tax group (the base in the company's currency)",
        );
        let name = match self.value("g", "account.tax.group", &["name"], 0) {
            Read::Sql(sql) => sql,
            _ => "NULL".into(),
        };
        let sign = "(CASE WHEN d.move_type IN ('out_invoice', 'out_receipt', 'in_refund') THEN -1 ELSE 1 END)";
        let leaf = |c: &str| c.strip_prefix(&nested.var).and_then(|r| r.strip_prefix('.')).map(str::to_owned);
        let refs = [doc];
        let mut select = Vec::new();
        for c in &nested.columns {
            let sql = match leaf(c).as_deref() {
                Some("group_name" | "group_label" | "name") => name.clone(),
                Some("tax_amount_currency") => format!("{sign} * i.amount_currency"),
                Some("tax_amount") => format!("{sign} * i.balance"),
                Some("base_amount_currency" | "display_base_amount_currency" | "base_amount" | "display_base_amount") => "abs(i.base)".into(),
                Some(other) => {
                    self.note(Severity::Warning, c, format!("a tax group has no `{other}` the query can read: the column reads NULL"));
                    "NULL".into()
                }
                None => self.column(c, &refs, shape),
            };
            select.push(format!("{sql} AS {}", ident(c)));
        }
        let joins: String = self.joins.iter().map(|j| format!("\n{}", j.sql)).collect();
        format!(
            "SELECT\n  {}\nFROM {} d\nJOIN LATERAL (SELECT x.tax_group_id, sum(x.amount_currency) AS amount_currency, sum(x.balance) AS balance, \
             sum(x.tax_base_amount) AS base FROM account_move_line x WHERE x.move_id = d.id AND x.display_type = 'tax' GROUP BY x.tax_group_id) i ON TRUE\n\
             LEFT JOIN account_tax_group g ON g.id = i.tax_group_id{joins}\nWHERE d.id IN (:{IDS_PARAMETER})\nORDER BY d.id, g.sequence, g.id",
            select.join(",\n  "),
            table(&doc.model)
        )
    }

    /// A nested loop's query: one row per item, with the document's (and line's) columns.
    fn nested(&mut self, shape: &Shape, nested: &NestedShape, doc: &Root, lines: Option<&Lines>) -> String {
        let mut from = format!("FROM {} d", table(&doc.model));
        let mut order = "d.id".to_owned();
        let mut roots = vec![doc.clone()];
        let parent = match lines.filter(|_| nested.under_line) {
            Some(l) => {
                from.push_str(&format!("\nJOIN {}", l.join.join()));
                order = format!("d.id, {}", l.order);
                roots.push(l.root.clone());
                l.root.clone()
            }
            None => doc.clone(),
        };
        let path: Vec<&str> = nested.source.split('.').skip(1).collect();
        if doc.model == "account.move" && !nested.under_line && path.join(".") == TAX_GROUPS {
            return self.tax_groups(shape, nested, doc);
        }
        let item = match self.records(&parent.alias, &parent.model, &path, "i", &nested.name) {
            Some((join, model)) => {
                from.push_str(&format!("\nJOIN {}", join.join()));
                order = format!("{order}, {}", self.order("i", &model));
                Some(Root { var: nested.var.clone(), alias: "i".into(), model })
            }
            None => None,
        };
        let found = item.is_some();
        roots.extend(item);
        let refs: Vec<&Root> = roots.iter().collect();
        let select: Vec<String> = nested.columns.iter().map(|c| format!("{} AS {}", self.column(c, &refs, shape), ident(c))).collect();
        for j in &self.joins {
            from.push_str(&format!("\n{}", j.sql));
        }
        // A loop the query can't follow reads no items, so its subreport stays hidden.
        let none = if found { "" } else { " AND FALSE" };
        format!("SELECT\n  {}\n{from}\nWHERE d.id IN (:{IDS_PARAMETER}){none}\nORDER BY {order}", select.join(",\n  "))
    }
}

/// `res.partner` → `"res_partner"` (Odoo's default table name).
fn table(model: &str) -> String {
    ident(&model.replace('.', "_"))
}

/// A quoted identifier.
fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// A quoted string literal.
fn literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn is_path(text: &str) -> bool {
    text.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(ttype: &str) -> OdooField {
        OdooField { ttype: ttype.into(), store: true, ..OdooField::default() }
    }

    fn m2o(relation: &str) -> OdooField {
        OdooField { relation: Some(relation.into()), ..f("many2one") }
    }

    /// An invoice and the models it reads, as `ir_model_fields` would describe them.
    fn schema() -> OdooSchema {
        let mut s = OdooSchema::default();
        for (name, field) in [
            ("name", f("char")),
            ("partner_id", m2o("res.partner")),
            ("company_id", m2o("res.company")),
            ("amount_total", f("monetary")),
            ("invoice_line_ids", OdooField { relation: Some("account.move.line".into()), relation_field: Some("move_id".into()), ..f("one2many") }),
            ("payment_ids", OdooField { relation: Some("account.payment".into()), relation_field: Some("move_id".into()), ..f("one2many") }),
            ("state", f("selection")),
            ("amount_total_words", OdooField { store: false, ..f("char") }),
        ] {
            s.insert("account.move", name, field);
        }
        for (name, field) in [
            ("name", f("text")),
            ("sequence", f("integer")),
            ("quantity", f("float")),
            ("display_type", f("selection")),
            ("product_id", m2o("product.product")),
            ("move_id", m2o("account.move")),
            (
                "tax_ids",
                OdooField {
                    relation: Some("account.tax".into()),
                    relation_table: Some("account_move_line_account_tax_rel".into()),
                    column1: Some("account_move_line_id".into()),
                    column2: Some("account_tax_id".into()),
                    ..f("many2many")
                },
            ),
        ] {
            s.insert("account.move.line", name, field);
        }
        s.insert("res.partner", "name", f("char"));
        s.insert("res.partner", "display_name", OdooField { store: false, ..f("char") });
        s.insert("res.company", "name", f("char"));
        s.insert("res.company", "logo", f("binary"));
        s.insert("res.company", "partner_id", m2o("res.partner"));
        s.insert("res.company", "street", OdooField { store: false, ..f("char") });
        s.insert("res.partner", "street", f("char"));
        s.insert("product.product", "product_tmpl_id", m2o("product.template"));
        s.insert("product.product", "name", OdooField { store: false, related: Some("product_tmpl_id.name".into()), ..f("char") });
        s.insert("product.template", "name", OdooField { translate: true, ..f("char") });
        s.insert("account.tax", "name", OdooField { translate: true, ..f("char") });
        s.insert("account.tax", "sequence", f("integer"));
        s.insert("account.tax", "tax_label", OdooField { store: false, ..f("char") });
        s.insert("account.payment", "amount", f("monetary"));
        s
    }

    fn invoice() -> Shape {
        Shape {
            doc_var: Some("o".into()),
            lines: Some(("line".into(), "o.invoice_line_ids".into())),
            columns: [
                "o.id",
                "o.name",
                "o.partner_id.name",
                "o.company_id.logo",
                "line.name",
                "line.quantity",
                "line.product_id.name",
                "line.tax_ids.tax_label",
                "line.discount",
                "o.company_id.street",
                "o.amount_total_words",
            ]
            .map(String::from)
            .to_vec(),
            joined: vec![("line.tax_ids.tax_label".into(), ", ".into())],
            datasets: vec![NestedShape {
                name: "o_payment_ids".into(),
                var: "pay".into(),
                source: "o.payment_ids".into(),
                columns: ["o.id", "pay.amount"].map(String::from).to_vec(),
                under_line: false,
            }],
            model: Some("account.move".into()),
        }
    }

    #[test]
    fn fields_become_joins_and_unreadable_ones_read_null_with_a_reason() {
        let options = QueryOptions { lang: "fr_FR".into(), ..QueryOptions::default() };
        let p = propose(&invoice(), &options, &schema()).unwrap();
        assert!(p.missing.is_empty(), "{:?}", p.missing);
        let q = &p.query;
        assert!(
            q.contains("FROM \"account_move\" d\nLEFT JOIN \"account_move_line\" l ON l.\"move_id\" = d.id AND l.\"display_type\" IN ('product'"),
            "{q}"
        );
        assert!(q.contains("LEFT JOIN \"res_partner\" t1 ON t1.id = d.\"partner_id\""), "{q}");
        assert!(q.contains("t1.\"name\" AS \"o.partner_id.name\""), "{q}");
        // A related field reads through its path; a translated one in the language, else English.
        assert!(q.contains("coalesce(t4.\"name\" ->> 'fr_FR', t4.\"name\" ->> 'en_US') AS \"line.product_id.name\""), "{q}");
        assert!(q.contains("a.res_model = 'res.company' AND a.res_field = 'logo'"), "{q}");
        // Taxes: a subquery, ordered by sequence; `tax_label` (Python) falls back on the name.
        assert!(q.contains("(SELECT string_agg((coalesce(t5.\"name\" ->> 'fr_FR', t5.\"name\" ->> 'en_US'))::text, ', ' ORDER BY t5.\"sequence\", t5.id) FROM \"account_move_line_account_tax_rel\" t5_rel"), "{q}");
        assert!(q.contains("NULL AS \"line.discount\""), "{q}");
        assert!(q.ends_with("WHERE d.id IN (:ids)\nORDER BY d.id, l.\"sequence\", l.id"), "{q}");
        assert!(
            p.diagnostics.iter().any(|d| d.location == "query: line.discount" && d.message.contains("no field `discount`")),
            "{:?}",
            p.diagnostics
        );
        assert!(p.diagnostics.iter().any(|d| d.message.contains("tax_label` is computed in Python")), "{:?}", p.diagnostics);
        // A company's address is its partner's; a computed text that isn't a label reads NULL.
        assert!(q.contains("LEFT JOIN \"res_partner\" t6 ON t6.id = t2.\"partner_id\""), "{q}");
        assert!(q.contains("t6.\"street\" AS \"o.company_id.street\""), "{q}");
        assert!(q.contains("NULL AS \"o.amount_total_words\""), "{q}");
        let (name, sub) = &p.datasets[0];
        assert_eq!(name, "o_payment_ids");
        assert!(sub.contains("JOIN \"account_payment\" i ON i.\"move_id\" = d.id"), "{sub}");
        assert!(sub.contains("i.\"amount\" AS \"pay.amount\""), "{sub}");
        // A loop the query can't follow reads no items.
        let mut shape = invoice();
        shape.datasets[0].source = "o.state".into();
        let p = propose(&shape, &options, &schema()).unwrap();
        assert!(p.datasets[0].1.contains("WHERE d.id IN (:ids) AND FALSE"), "{}", p.datasets[0].1);
        assert!(p.diagnostics.iter().any(|d| d.severity == Severity::Error && d.message.contains("not records to loop over")));
    }

    #[test]
    fn an_invoices_tax_totals_read_its_stored_amounts_and_tax_lines() {
        let options = QueryOptions { lang: "fr_FR".into(), ..QueryOptions::default() };
        let mut schema = schema();
        schema.insert("account.tax.group", "name", OdooField { translate: true, ..f("char") });
        let mut shape = invoice();
        shape.columns = [
            "o.id",
            "o.tax_totals",
            "o.tax_totals.total_amount_currency",
            "o.tax_totals.subtotals.name",
            "o.tax_totals.subtotals.base_amount_currency",
        ]
        .map(String::from)
        .to_vec();
        shape.datasets = vec![NestedShape {
            name: "o_tax_totals_subtotals_tax_groups".into(),
            var: "tax_group".into(),
            source: "o.tax_totals.subtotals.tax_groups".into(),
            columns: ["o.id", "tax_group.group_name", "tax_group.tax_amount_currency", "tax_group.display_base_amount_currency", "tax_group.odd"]
                .map(String::from)
                .to_vec(),
            under_line: false,
        }];
        let p = propose(&shape, &options, &schema).unwrap();
        assert!(p.missing.is_empty(), "{:?}", p.missing);
        let q = &p.query;
        assert!(q.contains("TRUE AS \"o.tax_totals\""), "{q}");
        assert!(q.contains("d.amount_total AS \"o.tax_totals.total_amount_currency\""), "{q}");
        assert!(q.contains("'Untaxed Amount' AS \"o.tax_totals.subtotals.name\""), "{q}");
        assert!(q.contains("d.amount_untaxed AS \"o.tax_totals.subtotals.base_amount_currency\""), "{q}");
        assert!(p.diagnostics.iter().any(|d| d.message.contains("computed in Python: the query reads `amount_total`")), "{:?}", p.diagnostics);
        let (name, sub) = &p.datasets[0];
        assert_eq!(name, "o_tax_totals_subtotals_tax_groups");
        assert!(sub.contains("x.move_id = d.id AND x.display_type = 'tax' GROUP BY x.tax_group_id) i ON TRUE"), "{sub}");
        assert!(sub.contains("coalesce(g.\"name\" ->> 'fr_FR', g.\"name\" ->> 'en_US') AS \"tax_group.group_name\""), "{sub}");
        // A customer invoice's taxes are credits: shown positive.
        assert!(sub.contains("(CASE WHEN d.move_type IN ('out_invoice', 'out_receipt', 'in_refund') THEN -1 ELSE 1 END) * i.amount_currency AS \"tax_group.tax_amount_currency\""), "{sub}");
        assert!(sub.contains("abs(i.base) AS \"tax_group.display_base_amount_currency\""), "{sub}");
        assert!(sub.contains("NULL AS \"tax_group.odd\"") && sub.contains("d.id AS \"o.id\""), "{sub}");
        assert!(sub.ends_with("WHERE d.id IN (:ids)\nORDER BY d.id, g.sequence, g.id"), "{sub}");
        // The tax group's model is asked for when it isn't known yet.
        let p = propose(&shape, &options, &self::schema()).unwrap();
        assert_eq!(p.missing, ["account.tax.group"]);
    }

    #[test]
    fn missing_models_are_asked_for_one_join_at_a_time() {
        let mut full = schema();
        let mut s = OdooSchema::default();
        let mut rounds = 0;
        let shape = invoice();
        let p = loop {
            let p = propose(&shape, &QueryOptions::default(), &s).unwrap();
            if p.missing.is_empty() {
                break p;
            }
            rounds += 1;
            assert!(rounds < 8, "{:?}", p.missing);
            for m in &p.missing {
                for (name, field) in full.models.remove(m).unwrap_or_default() {
                    s.insert(m, &name, field);
                }
                s.models.entry(m.clone()).or_default();
            }
        };
        assert!(rounds >= 3, "document, then its line and partner, then product and template");
        assert!(p.query.contains("t3.\"name\" ->> 'en_US'") || p.query.contains("->> 'en_US' AS \"line.product_id.name\""), "{}", p.query);
    }

    #[test]
    fn sections_are_numbered_by_window_counts() {
        let mut shape = invoice();
        shape.columns = ["o.id", "line.name", "line.display_type", "line.section_number", "line.subsection_number"].map(String::from).to_vec();
        shape.datasets.clear();
        let q = propose(&shape, &QueryOptions::default(), &schema()).unwrap().query;
        assert!(q.contains("count(*) FILTER (WHERE l.\"display_type\" = 'line_section') OVER w AS \"line.section_number\""), "{q}");
        assert!(q.contains("PARTITION BY q.\"o.id\" ORDER BY q.rc_order), 0) AS \"line.subsection_number\""), "{q}");
        assert!(q.contains("WINDOW w AS (PARTITION BY d.id ORDER BY l.\"sequence\", l.id)\n) q\nORDER BY q.rc_order"), "{q}");
        assert!(q.starts_with("SELECT\n  q.\"o.id\",\n  q.\"line.name\""), "{q}");
    }

    #[test]
    fn lines_from_python_use_the_documented_field_or_the_one_given() {
        let mut shape = invoice();
        shape.lines = Some(("line".into(), "lines_to_report".into()));
        let p = propose(&shape, &QueryOptions::default(), &schema()).unwrap();
        assert!(p.query.contains("LEFT JOIN \"account_move_line\" l"), "{}", p.query);
        assert!(p.diagnostics.iter().any(|d| d.message.contains("read from `invoice_line_ids`")));
        let mut other = shape.clone();
        other.model = Some("res.partner".into());
        let p = propose(&other, &QueryOptions::default(), &schema()).unwrap();
        assert!(p.diagnostics.iter().any(|d| d.severity == Severity::Error && d.message.contains("with `lines`")), "{:?}", p.diagnostics);
    }

    #[test]
    fn hostile_names_are_quoted_and_bad_options_refused() {
        let mut s = schema();
        s.insert("account.move", "x\"y", f("char"));
        let mut shape = invoice();
        shape.columns = vec!["o.x\"y".into()];
        shape.datasets.clear();
        let q = propose(&shape, &QueryOptions::default(), &s).unwrap().query;
        assert!(q.contains("d.\"x\"\"y\" AS \"o.x\"\"y\""), "{q}");
        assert!(propose(&shape, &QueryOptions { lang: "fr'; drop".into(), ..QueryOptions::default() }, &s).is_err());
        assert!(propose(&shape, &QueryOptions { model: Some("Robert'); --".into()), ..QueryOptions::default() }, &s).is_err());
        assert!(propose(&Shape::default(), &QueryOptions::default(), &s).is_err());
        assert_eq!(metadata_query(&["res.partner".into(), "x'); drop".into()]).matches("IN ('res.partner')").count(), 1);
    }

    #[test]
    fn metadata_rows_fill_the_schema() {
        use reportcraft_core::{Column, FieldType};
        let names = ["model", "name", "ttype", "relation", "store", "related", "translate"];
        let columns = names.iter().map(|n| Column { name: (*n).into(), field_type: FieldType::Text }).collect();
        let t = |s: &str| Value::Text(s.into());
        let rows = RowSet {
            columns,
            rows: vec![
                vec![t("res.partner"), t("name"), t("char"), Value::Null, Value::Bool(true), Value::Null, Value::Null],
                vec![t("res.partner"), t("country_id"), t("many2one"), t("res.country"), Value::Bool(true), Value::Null, Value::Null],
                vec![t("res.country"), t("name"), t("char"), Value::Null, Value::Bool(true), Value::Null, t("standard")],
            ],
        };
        let mut s = OdooSchema::default();
        s.add(&["res.partner".into(), "res.country".into(), "nope".into()], &rows).unwrap();
        assert!(s.has_model("res.country") && !s.has_model("nope"));
        assert!(matches!(s.lookup("res.country", "name"), Lookup::Found(f) if f.translate));
        assert!(matches!(s.lookup("nope", "x"), Lookup::Unknown));
        assert!(matches!(s.lookup("other", "x"), Lookup::Pending));
    }
}
