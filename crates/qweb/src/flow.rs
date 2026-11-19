//! Lays a QWeb template's HTML out as bands: a top-to-bottom flow of lines, each line the inline
//! text and fields of a block (`p`, `div`, `h2`…), tables as rows of equal columns. The document
//! loop (`t-foreach="docs"`) becomes a group with a page break, the loop over its lines the detail
//! band; what comes before the lines goes in the group header, what follows in its footer.
//! Another loop over a document's or a line's records (`o.payment_ids`, `move.move_line_ids`)
//! becomes a subreport over a dataset of its own, linked to the document (and the line).

use reportcraft_core::FieldType;
use reportcraft_core::limits::MAX_ELEMENTS_PER_BAND;
use reportcraft_model::{Align, Band, Content, Element, Format, ImageFit, Report, Severity, SubreportLink, TextStyle};
use roxmltree::Node;

use crate::expr;
use crate::letterhead::Layout;

/// The width laid out on: A4 less two 15 mm margins.
pub(crate) const CONTENT_MM: f64 = 180.0;
/// Values Odoo gives every report's template (and the layouts' own), never unset there.
const REPORT_VALUES: [&str; 21] = [
    "docs",
    "doc_ids",
    "doc_model",
    "data",
    "user",
    "res_company",
    "company",
    "time",
    "context_timestamp",
    "report_type",
    "web_base_url",
    "env",
    "request",
    "lang",
    "datetime",
    "relativedelta",
    "json",
    "is_html_empty",
    "html2plaintext",
    "to_text",
    "float_round",
];
/// Deepest element nesting followed; deeper content is left out with a warning.
const MAX_DEPTH: usize = 64;
/// Millimetres per point.
const MM_PER_PT: f64 = 25.4 / 72.0;
/// Width of a character of text, as a share of the font size: an average for proportional fonts.
const CHAR_EM: f64 = 0.55;
/// Room given to a field that isn't last on its line.
const FIELD_MM: f64 = 40.0;

/// Where the flow writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    Header,
    Detail,
    Footer,
    /// The band of the nested loop being read.
    Nested,
}

/// What a table is read as, in order: a row (or a `t-set` between rows) with the calls it is
/// in, or entering and leaving a called template.
enum TableItem<'a, 'i> {
    Row(Node<'a, 'i>, Vec<Node<'a, 'i>>),
    Enter(Node<'a, 'i>, String),
    Leave(Node<'a, 'i>),
}

/// A finding while reading, located by line.
pub(crate) struct Finding {
    pub severity: Severity,
    pub line: u32,
    pub message: String,
}

/// The loop a column varies with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    Doc,
    Line,
    /// A nested loop's item.
    Item,
}

/// A column the template reads, with the type its name suggests (for sample data and formats).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Field {
    pub name: String,
    pub field_type: FieldType,
    pub level: Level,
    /// The first text a condition compares it with (`o.state == 'posted'`), its sample value.
    pub example: Option<String>,
}

/// The rows a nested loop reads: one per item, the document's (and the line's) columns repeated,
/// linked to the main data by `links` (the document's id, and the line's).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Dataset {
    pub name: String,
    pub fields: Vec<Field>,
    pub links: Vec<String>,
    pub under_line: bool,
    /// What the loop goes over (`o.payment_ids`, `line.lot_ids`) and its variable.
    pub source: String,
    pub var: String,
}

/// A nested loop being read: its band, the band it goes into, and its data.
struct Nested {
    node: roxmltree::NodeId,
    /// The table whose rows it repeats, if it is a table row loop.
    table: Option<roxmltree::NodeId>,
    outer: Target,
    band: Band,
    dataset: Dataset,
    vars: (String, Option<String>, String),
}

/// A column of a Bootstrap grid row being read: its lines go from `x`, `width` wide, the next one
/// at `bottom`.
#[derive(Clone, Copy)]
struct Frame {
    x: f64,
    width: f64,
    bottom: f64,
    /// The band it is in; a nested loop's band read inside it is a subreport as wide as it.
    target: Target,
}

/// A block of HTML a `t-set` names (`<t t-set="address">…</t>`), shown where the name is written
/// out, with the condition it was set under.
#[derive(Clone)]
struct Slot<'a, 'i> {
    name: String,
    node: Node<'a, 'i>,
    condition: Option<String>,
}

/// One piece of a line of text.
enum Piece {
    Text(String, TextStyle),
    Field(String, TextStyle, Option<Format>),
    /// An expression with no ReportCraft meaning, shown as written.
    Raw(String, TextStyle),
    /// A formula the converter wrote (a section's total).
    Formula(String, TextStyle, Option<Format>),
}

impl Piece {
    fn style(&self) -> &TextStyle {
        match self {
            Piece::Text(_, s) | Piece::Field(_, s, _) | Piece::Raw(_, s) | Piece::Formula(_, s, _) => s,
        }
    }
}

/// The loop variables in scope and the condition elements are shown under.
#[derive(Clone, Default)]
pub(crate) struct Scope {
    pub doc_var: Option<String>,
    pub line_var: Option<String>,
    /// A nested loop's variable.
    pub item_var: Option<String>,
    pub condition: Option<String>,
    pub style: Style,
}

/// Inherited text style.
#[derive(Clone, Copy)]
pub(crate) struct Style {
    pub size_pt: f64,
    pub bold: bool,
    pub align: Align,
}

impl Default for Style {
    fn default() -> Self {
        Self { size_pt: 10.0, bold: false, align: Align::Left }
    }
}

impl Style {
    fn text(self) -> TextStyle {
        TextStyle { size_pt: self.size_pt, bold: self.bold, align: self.align, color: None, background: None }
    }

    /// The style inside an element: its tag (headings, `strong`, `th`, `small`) and Bootstrap's
    /// classes (`text-end`, `text-center`, `fw-bold`).
    fn inside(mut self, node: Node<'_, '_>) -> Self {
        match node.tag_name().name() {
            "h1" => (self.size_pt, self.bold) = (20.0, true),
            "h2" => (self.size_pt, self.bold) = (16.0, true),
            "h3" => (self.size_pt, self.bold) = (14.0, true),
            "h4" => (self.size_pt, self.bold) = (12.0, true),
            "h5" | "h6" | "strong" | "b" | "th" => self.bold = true,
            "small" => self.size_pt = (self.size_pt * 0.8).max(6.0),
            _ => {}
        }
        for class in print_classes(node).split_whitespace() {
            match class {
                "text-end" | "text-right" => self.align = Align::Right,
                "text-center" => self.align = Align::Center,
                "text-start" | "text-left" => self.align = Align::Left,
                "fw-bold" | "fw-bolder" | "font-weight-bold" => self.bold = true,
                "fw-normal" => self.bold = false,
                _ => {}
            }
        }
        self
    }
}

/// The bands being built.
pub(crate) struct Builder<'a, 'i> {
    /// The file's templates by name, for `t-call`s to them, and the calls being followed.
    library: Vec<(String, Node<'a, 'i>)>,
    calls: Vec<String>,
    /// The loop element whose rows are the detail band.
    detail_loop: Option<roxmltree::NodeId>,
    /// Names given by `t-set` to paths, latest last.
    aliases: Vec<(String, String)>,
    /// Names given by `t-set` to conditions (`is_section = line.display_type == 'line_section'`),
    /// as formulas, latest last.
    conditions: Vec<(String, String)>,
    /// Names given by `t-set` to Python that isn't converted, with that Python, latest last.
    unread: Vec<(String, String)>,
    /// Columns that hold records' values joined into one text (`line.tax_ids.tax_label`), with
    /// the separator the template joins them with.
    pub joined: Vec<(String, String)>,
    pub header: Band,
    pub detail: Band,
    pub footer: Band,
    pub target: Target,
    pub fields: Vec<Field>,
    pub findings: Vec<Finding>,
    /// Found the document loop and the line loop.
    pub doc_loop: Option<String>,
    pub line_loop: bool,
    /// The line loop's variable and what it loops over (`o.invoice_line_ids`, or a `t-set` name
    /// for Python such as `lines_to_report`).
    pub lines: Option<(String, String)>,
    /// The page wrapper the template calls, with the document variable its company is read from.
    pub letterhead: Option<(Layout, String)>,
    /// The nested loop being read, and those read.
    nested: Option<Nested>,
    pub datasets: Vec<Dataset>,
    /// The line of text being gathered, each piece with the condition it shows under.
    pending: Vec<(Piece, Option<String>)>,
    /// The grid column being read, if any.
    frame: Option<Frame>,
    /// Blocks named by `t-set`, latest last, and whether the page layout's blocks were placed.
    slots: Vec<Slot<'a, 'i>>,
    layout_blocks: bool,
    /// Names a condition read that nothing sets (read as false), and those already reported.
    undefined: std::cell::RefCell<Vec<String>>,
    undefined_said: Vec<String>,
    /// A table cell gathers its pieces here, with the condition each shows under, instead of
    /// laying out lines.
    cell: Option<Vec<(Piece, Option<String>)>>,
    /// The XML line of the node being read.
    line: u32,
    full_bands: bool,
}

impl<'a, 'i> Builder<'a, 'i> {
    pub(crate) fn new(library: Vec<(String, Node<'a, 'i>)>) -> Self {
        Self {
            library,
            calls: Vec::new(),
            aliases: Vec::new(),
            conditions: Vec::new(),
            unread: Vec::new(),
            joined: Vec::new(),
            detail_loop: None,
            header: Band::default(),
            detail: Band::default(),
            footer: Band::default(),
            target: Target::Header,
            fields: Vec::new(),
            findings: Vec::new(),
            doc_loop: None,
            line_loop: false,
            lines: None,
            letterhead: None,
            nested: None,
            datasets: Vec::new(),
            pending: Vec::new(),
            frame: None,
            slots: Vec::new(),
            layout_blocks: false,
            undefined: std::cell::RefCell::new(Vec::new()),
            undefined_said: Vec::new(),
            cell: None,
            line: 0,
            full_bands: false,
        }
    }

    /// Whether any band has an element.
    pub(crate) fn has_content(&self) -> bool {
        [&self.header, &self.detail, &self.footer].iter().any(|b| !b.elements.is_empty())
    }

    /// Marks the template being read, so a call back to it stops.
    pub(crate) fn enter(&mut self, name: &str) {
        self.calls.push(name.to_owned());
    }

    fn note(&mut self, severity: Severity, message: impl Into<String>) {
        self.findings.push(Finding { severity, line: self.line, message: message.into() });
    }

    pub(crate) fn warn(&mut self, message: impl Into<String>) {
        self.note(Severity::Warning, message);
    }

    fn band(&mut self) -> &mut Band {
        match self.target {
            Target::Header => &mut self.header,
            Target::Detail => &mut self.detail,
            Target::Footer => &mut self.footer,
            Target::Nested => match self.nested.as_mut() {
                Some(n) => &mut n.band,
                None => &mut self.footer,
            },
        }
    }

    /// Adds an element at the bottom of the current band, `height` tall from `top`.
    fn push(&mut self, element: Element) {
        let band = self.band();
        if band.elements.len() >= MAX_ELEMENTS_PER_BAND {
            if !self.full_bands {
                self.full_bands = true;
                self.warn(format!("a band holds at most {MAX_ELEMENTS_PER_BAND} elements: the rest is left out"));
            }
            return;
        }
        let end = element.y_mm + element.height_mm;
        band.height_mm = band.height_mm.max(end);
        band.elements.push(element);
        let target = self.target;
        if let Some(f) = self.frame.as_mut().filter(|f| f.target == target) {
            f.bottom = f.bottom.max(end);
        }
    }

    /// Where the next line goes: below the grid column being read, or the band.
    fn bottom(&mut self) -> f64 {
        match self.frame {
            Some(f) if f.target == self.target => f.bottom,
            _ => self.band().height_mm,
        }
    }

    /// Moves where the next line goes (room after a paragraph or a table).
    fn set_bottom(&mut self, y: f64) {
        let target = self.target;
        if let Some(f) = self.frame.as_mut().filter(|f| f.target == target) {
            f.bottom = y;
            let band = self.band();
            band.height_mm = band.height_mm.max(y);
        } else {
            self.band().height_mm = y;
        }
    }

    /// The left edge and width lines are laid in: the grid column's, or the page's.
    fn area(&self) -> (f64, f64) {
        match self.frame {
            Some(f) if f.target == self.target => (f.x, f.width),
            // A nested loop's band, read inside the column: its subreport is the column's width.
            Some(f) => (0.0, f.width),
            None => (0.0, CONTENT_MM),
        }
    }

    /// Reads `node` (or, when it is being read already, its children) in a column from `x`,
    /// `width` wide, starting at `top`; returns where the column ends.
    fn in_frame(&mut self, (node, itself): (Node<'a, 'i>, bool), (x, width): (f64, f64), top: f64, scope: &Scope, depth: usize) -> f64 {
        let outer = self.frame;
        self.frame = Some(Frame { x, width, bottom: top, target: self.target });
        if itself {
            self.walk(node, scope, depth + 1);
        } else {
            self.children(node, scope, depth);
        }
        self.flush(scope);
        let bottom = self.frame.map_or(top, |f| f.bottom);
        self.frame = outer;
        bottom
    }

    /// Records a column the template reads, in the nested loop's data while one is read; the
    /// first use decides which loop it varies with.
    fn field(&mut self, name: &str, scope: &Scope) -> Option<Format> {
        let root = name.split('.').next();
        let level = if scope.item_var.as_deref().is_some() && scope.item_var.as_deref() == root {
            Level::Item
        } else if scope.line_var.as_deref().is_some() && scope.line_var.as_deref() == root {
            Level::Line
        } else {
            Level::Doc
        };
        let fields = match (self.target, self.nested.as_mut()) {
            (Target::Nested, Some(n)) => &mut n.dataset.fields,
            // A condition on a nested loop's row, read before the loop opens: its rows carry it.
            _ if level == Level::Item => return None,
            _ => &mut self.fields,
        };
        add_field(fields, name, level);
        if self.joined.iter().any(|(j, _)| j.eq_ignore_ascii_case(name)) {
            if let Some(f) = fields.iter_mut().find(|f| f.name.eq_ignore_ascii_case(name)) {
                f.field_type = FieldType::Text;
            }
            return None;
        }
        money(name).then_some(Format::Number { decimals: 2, grouping: true })
    }

    /// Fields a condition compares with a text (`[o.state] = "posted"`) are text, and that text
    /// is their sample value (the first one met), so the sample shows the branch it opens.
    fn compared(&mut self, formula: &str) {
        for (open, _) in formula.match_indices('[') {
            let rest = &formula[open + 1..];
            let Some(close) = rest.find(']') else { continue };
            let after = &rest[close + 1..];
            let Some(literal) = after.strip_prefix(" = \"").or_else(|| after.strip_prefix(" <> \"")) else { continue };
            let Some(end) = literal.find('"') else { continue };
            let (name, text) = (&rest[..close], &literal[..end]);
            let nested = self.nested.as_mut().map(|n| &mut n.dataset.fields);
            for f in self.fields.iter_mut().chain(nested.into_iter().flatten()).filter(|f| f.name.eq_ignore_ascii_case(name)) {
                f.field_type = FieldType::Text;
                f.example.get_or_insert_with(|| text.to_owned());
            }
        }
    }

    /// Whether a loop over `each` can be a nested loop: a path from the document's variable, or
    /// the line's. Over anything else (Python, a `t-set` list) its variable isn't a
    /// record, and conditions on it aren't fields.
    fn nestable(each: &str, scope: &Scope) -> bool {
        let root = each.split('.').next().unwrap_or(each);
        expr::path(each).is_some() && each.contains('.') && (scope.doc_var.as_deref() == Some(root) || scope.line_var.as_deref() == Some(root))
    }

    /// Starts reading a nested loop over `each` (a path from the document's or the line's
    /// variable) into a band of its own; `false` when it can't be one (another nested loop is
    /// being read, the band already holds one, it isn't a path of the document or line).
    fn open_nested(&mut self, node: Node<'a, 'i>, var: &str, each: &str, scope: &Scope, table: Option<roxmltree::NodeId>) -> bool {
        let Some(doc) = scope.doc_var.clone() else { return false };
        let root = each.split('.').next().unwrap_or(each);
        let line = scope.line_var.clone().filter(|l| l == root && self.target == Target::Detail);
        if self.nested.is_some() || self.cell.is_some() || expr::path(each).is_none() || !each.contains('.') || (root != doc && line.is_none()) {
            return false;
        }
        if self.band().elements.iter().any(|e| matches!(e.content, Content::Subreport { .. })) {
            self.warn("one loop per band is repeated as a subreport");
            return false;
        }
        self.flush(scope);
        let mut links = vec![(format!("{doc}.id"), Level::Doc)];
        if let Some(l) = &line {
            links.push((format!("{l}.id"), Level::Line));
        }
        let mut fields = Vec::new();
        for (name, level) in &links {
            add_field(&mut self.fields, name, *level);
            add_field(&mut fields, name, *level);
        }
        let mut name: String = each.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).take(56).collect();
        let base = name.clone();
        for n in 2.. {
            if !self.datasets.iter().any(|d| d.name == name) {
                break;
            }
            name = format!("{base}_{n}");
        }
        let dataset = Dataset {
            name,
            fields,
            links: links.into_iter().map(|(n, _)| n).collect(),
            under_line: line.is_some(),
            source: each.to_owned(),
            var: var.to_owned(),
        };
        let vars = (doc, line, var.to_owned());
        self.nested = Some(Nested { node: node.id(), table, outer: self.target, band: Band::default(), dataset, vars });
        self.target = Target::Nested;
        true
    }

    /// Ends the nested loop being read: its band becomes a subreport at the bottom of the band it
    /// was in, linked to the document (and the line).
    fn close_nested(&mut self) {
        if self.nested.is_none() {
            return;
        }
        self.flush(&Scope::default());
        let Some(mut n) = self.nested.take() else { return };
        self.target = n.outer;
        // Conditions may read the document's columns: the nested rows carry them.
        let (doc, line, item) = &n.vars;
        for condition in n.band.elements.iter().filter_map(|e| e.visible_if.as_deref()) {
            for f in condition.split('[').skip(1).filter_map(|f| f.split(']').next()) {
                let root = f.split('.').next().unwrap_or(f);
                let level = if root == item {
                    Level::Item
                } else if line.as_deref() == Some(root) {
                    Level::Line
                } else if root == doc {
                    Level::Doc
                } else {
                    continue;
                };
                add_field(&mut n.dataset.fields, f, level);
            }
        }
        if n.band.elements.is_empty() {
            return;
        }
        // A condition every element shares hides the row, which then takes no room.
        let shared = n.band.elements.first().and_then(|e| e.visible_if.clone());
        if shared.is_some() && n.band.elements.iter().all(|e| e.visible_if == shared) {
            for e in &mut n.band.elements {
                e.visible_if = None;
            }
            n.band.visible_if = shared;
        }
        let height = n.band.height_mm.max(1.0);
        let mut child = Report::new(&n.dataset.name);
        child.detail = n.band;
        let links = n.dataset.links.iter().map(|l| SubreportLink { parent: l.clone(), child: l.clone() }).collect();
        let content = Content::Subreport { report: Box::new(child), dataset: n.dataset.name.clone(), links, hide_when_empty: true };
        let y = self.bottom();
        let (x, width) = self.area();
        self.push(element(x, y, width, height, Style::default().text(), content, &Scope::default()));
        self.datasets.push(n.dataset);
    }

    /// Walks a node and its children.
    pub(crate) fn walk(&mut self, node: Node<'a, 'i>, scope: &Scope, depth: usize) {
        if node.is_text() {
            self.text(node.text().unwrap_or(""), scope);
            return;
        }
        if !node.is_element() {
            return;
        }
        self.line = node.document().text_pos_at(node.range().start).row;
        if depth > MAX_DEPTH {
            self.warn(format!("elements nested more than {MAX_DEPTH} deep are left out"));
            return;
        }
        if hidden_in_print(node) {
            return;
        }
        let mut scope = scope.clone();
        scope.style = scope.style.inside(node);
        if !self.conditions(node, &mut scope) {
            return;
        }
        if let Some(each) = node.attribute("t-foreach") {
            self.foreach(node, each, scope, depth);
            return;
        }
        if let Some(call) = node.attribute("t-call")
            && self.call(call, node, &scope, depth)
        {
            return;
        }
        if let Some(var) = node.attribute("t-set") {
            let body = node.children().any(|c| c.is_element() || c.text().is_some_and(|t| !t.trim().is_empty()));
            if node.attribute("t-value").is_none() && body {
                self.forget(var);
                self.slots.push(Slot { name: var.to_owned(), node, condition: scope.condition.clone() });
            } else {
                self.set(var, node.attribute("t-value"), &scope);
            }
            return;
        }
        if let Some(value) = ["t-field", "t-out", "t-esc", "t-raw"].iter().find_map(|a| node.attribute(*a)) {
            // A block a `t-set` named (`t-out="address"`, `t-out="information_block or None"`).
            if let Some(slot) = self.slot(value.split(" or ").next().unwrap_or(value).trim()) {
                let block = is_block(node.tag_name().name());
                if block {
                    self.flush(&scope);
                }
                self.show_slot(&slot, &scope, depth);
                if block {
                    self.flush(&scope);
                }
                return;
            }
            let value = &expr::keys(value);
            // A heading or paragraph that is a field is a line of its own.
            let block = is_block(node.tag_name().name());
            if block {
                self.flush(&scope);
            }
            self.value(value, node, &scope);
            if block {
                self.flush(&scope);
            }
            return;
        }
        self.html(node, &scope, depth);
    }

    /// Applies `t-if`, `t-elif` and `t-else` to the scope (a branch shows when the ones before it
    /// don't); `false` when the element is left out because its condition can't be converted.
    fn conditions(&mut self, node: Node<'a, 'i>, scope: &mut Scope) -> bool {
        let formula = if node.has_attribute("t-elif") || node.has_attribute("t-else") {
            match self.branch(node, scope) {
                Some(f) => f,
                None => {
                    self.warn("a `t-elif` or `t-else` branch is left out: a condition before it isn't converted");
                    return false;
                }
            }
        } else {
            let Some(test) = node.attribute("t-if") else { return true };
            match self.test(test, scope) {
                Some(f) => f,
                None => {
                    self.undefined.borrow_mut().clear();
                    self.warn(format!("condition `{}` isn't converted: its content is always shown", collapse(test)));
                    return true;
                }
            }
        };
        self.say_undefined();
        for field in formula.split('[').skip(1).filter_map(|f| f.split(']').next()) {
            self.field(field, scope);
        }
        self.compared(&formula);
        scope.condition = Some(match scope.condition.take() {
            Some(outer) => format!("AND({outer}, {formula})"),
            None => formula,
        });
        true
    }

    /// A `t-elif` or `t-else` branch's formula: none of the conditions before it, and its own.
    fn branch(&mut self, node: Node<'a, 'i>, scope: &Scope) -> Option<String> {
        let mut parts = Vec::new();
        let mut sibling = node.prev_sibling_element();
        loop {
            let s = sibling?;
            let (test, first) = match (s.attribute("t-elif"), s.attribute("t-if")) {
                (Some(t), _) => (t, false),
                (None, Some(t)) => (t, true),
                (None, None) => return None,
            };
            parts.push(format!("NOT({})", self.test(test, scope)?));
            if first {
                break;
            }
            sibling = s.prev_sibling_element();
        }
        parts.reverse();
        if let Some(test) = node.attribute("t-elif") {
            parts.push(self.test(test, scope)?);
        }
        Some(if let [one] = parts.as_slice() { one.clone() } else { format!("AND({})", parts.join(", ")) })
    }

    /// A condition as a formula, its fields read through the `t-set` names.
    fn test(&self, test: &str, scope: &Scope) -> Option<String> {
        let named = |name: &str| {
            self.conditions.iter().rev().find(|(n, _)| n == name).map(|(_, f)| f.clone()).or_else(|| {
                self.is_undefined(name, scope).then(|| {
                    self.undefined.borrow_mut().push(name.to_owned());
                    "FALSE".to_owned()
                })
            })
        };
        let formula = expr::condition_with(&expr::keys(&collapse(test)), &named)?;
        self.read_fields(&formula, scope)
    }

    /// A bare name nothing in the template sets (no `t-set`, loop or call passes it) and that
    /// isn't one of the values Odoo gives every report: Odoo reads it as `None`, so a condition
    /// on it is false (`print_with_payments`, `proforma` when the template printed doesn't set
    /// them).
    fn is_undefined(&self, name: &str, scope: &Scope) -> bool {
        let identifier =
            name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        identifier
            && !matches!(name, "True" | "False" | "None")
            && !REPORT_VALUES.contains(&name)
            && ![&scope.doc_var, &scope.line_var, &scope.item_var].iter().any(|v| v.as_deref() == Some(name))
            && !self.aliases.iter().any(|(n, _)| n == name)
            && !self.conditions.iter().any(|(n, _)| n == name)
            && !self.unread.iter().any(|(n, _)| n == name)
            && !self.slots.iter().any(|s| s.name == name)
            && self.doc_loop.as_deref() != Some(name)
    }

    /// Says once per name that a condition read it as false.
    fn say_undefined(&mut self) {
        let names: Vec<String> = self.undefined.borrow_mut().drain(..).collect();
        for name in names {
            if !self.undefined_said.contains(&name) {
                self.warn(format!("`{name}` isn't set by the template: read as false, as Odoo reads a name nothing sets"));
                self.undefined_said.push(name);
            }
        }
    }

    /// A formula with its fields read through the `t-set` names, when each is a field of the
    /// documents or their lines (`None` otherwise).
    fn read_fields(&self, formula: &str, scope: &Scope) -> Option<String> {
        let mut out = String::new();
        let mut rest = formula;
        while let Some(open) = rest.find('[') {
            let close = rest[open..].find(']').map(|c| open + c)?;
            let field = self.resolve(&rest[open + 1..close]);
            if !self.known(&field, scope) {
                return None;
            }
            out.push_str(&rest[..=open]);
            out.push_str(&field);
            out.push(']');
            rest = &rest[close + 1..];
        }
        out.push_str(rest);
        Some(out)
    }

    /// A path with a `t-set` name at its start replaced by what it names.
    fn resolve(&self, path: &str) -> String {
        let (head, tail) = path.split_once('.').map_or((path, None), |(h, t)| (h, Some(t)));
        match self.aliases.iter().rev().find(|(name, _)| name == head) {
            Some((_, target)) => tail.map_or_else(|| target.clone(), |t| format!("{target}.{t}")),
            None => path.to_owned(),
        }
    }

    /// `t-set`: a name for a path (`partner = o.partner_id`, `lines = o.move_ids.filtered(…)`) is
    /// remembered and read through; filters and sorts aren't applied, which is reported. A name
    /// for a condition (`is_section = line.display_type == 'line_section'`) is remembered as its
    /// formula, for the `t-if`s that test it.
    fn set(&mut self, var: &str, value: Option<&str>, scope: &Scope) {
        let Some(value) = value.map(|v| expr::keys(&collapse(v))) else {
            self.forget(var);
            self.warn(format!("`t-set=\"{var}\"` with a body is ignored"));
            return;
        };
        if let Some(column) = self.join(&value, scope) {
            self.forget(var);
            self.aliases.push((var.to_owned(), column));
            return;
        }
        let cut = [".filtered(", ".sorted(", ".with_context(", ".sudo(", ".with_company("].iter().filter_map(|m| value.find(m)).min();
        let (base, tail) = cut.map_or((value.as_str(), ""), |i| value.split_at(i));
        let Some(base) = expr::path(base) else {
            let condition = self.test(&value, scope);
            self.forget(var);
            match condition {
                Some(formula) => self.conditions.push((var.to_owned(), formula)),
                None => {
                    self.unread.push((var.to_owned(), value.clone()));
                    self.warn(format!("`t-set=\"{var}\"` is ignored"));
                }
            }
            return;
        };
        let target = self.resolve(base);
        if target == var {
            return;
        }
        self.forget(var);
        if tail.contains(".filtered(") || tail.contains(".sorted(") {
            self.warn(format!("`t-set=\"{var}\"`: filters and sorts aren't applied, `{var}` reads every record of `{target}`"));
        }
        self.aliases.push((var.to_owned(), target));
    }

    /// Drops what a name stood for: it is being set again.
    fn forget(&mut self, var: &str) {
        self.aliases.retain(|(n, _)| n != var);
        self.conditions.retain(|(n, _)| n != var);
        self.unread.retain(|(n, _)| n != var);
        self.slots.retain(|s| s.name != var);
    }

    /// The block a `t-set` named `name`, if one did.
    fn slot(&self, name: &str) -> Option<Slot<'a, 'i>> {
        self.slots.iter().rev().find(|s| s.name == name).cloned()
    }

    /// Reads a named block where it is shown, under the condition it was set under too.
    fn show_slot(&mut self, slot: &Slot<'a, 'i>, scope: &Scope, depth: usize) {
        let mut inner = scope.clone();
        if let Some(c) = &slot.condition {
            inner.condition = Some(match inner.condition.take() {
                Some(outer) if outer != *c => format!("AND({outer}, {c})"),
                _ => c.clone(),
            });
        }
        self.children(slot.node, &inner, depth + 1);
    }

    /// The blocks a page layout shows above the document (`web.external_layout`'s `address`,
    /// `information_block` and `layout_document_title`, passed by name or set before the call),
    /// placed as Odoo's layout places them: the information block on the left half, the address
    /// in the right five twelfths, then the title as a heading.
    fn layout_blocks(&mut self, call: Node<'a, 'i>, scope: &Scope, depth: usize) {
        if self.layout_blocks {
            return;
        }
        self.layout_blocks = true;
        let find = |b: &Self, param: &str| call.attribute(param).and_then(|v| b.slot(v.trim())).or_else(|| b.slot(param));
        let (info, address, title) = (find(self, "information_block"), find(self, "address"), find(self, "layout_document_title"));
        self.flush(scope);
        if info.is_some() || address.is_some() {
            let (x, width) = self.area();
            let top = self.bottom();
            let outer = self.frame;
            let mut bottom = top;
            for (slot, left, w) in [(info, x, width * 6.0 / 12.0), (address, x + width * 7.0 / 12.0, width * 5.0 / 12.0)] {
                let Some(slot) = slot else { continue };
                self.frame = Some(Frame { x: left, width: w, bottom: top, target: self.target });
                self.show_slot(&slot, scope, depth);
                self.flush(scope);
                bottom = bottom.max(self.frame.map_or(top, |f| f.bottom));
            }
            self.frame = outer;
            self.set_bottom(bottom + 4.0);
        }
        if let Some(slot) = title {
            let mut heading = scope.clone();
            (heading.style.size_pt, heading.style.bold) = (16.0, true);
            self.show_slot(&slot, &heading, depth);
            self.flush(&heading);
            let after = self.bottom() + 2.0;
            self.set_bottom(after);
        }
    }

    /// Follows a `t-call`: a template of the same file is read in place (`true`, the element's own
    /// content is then what Odoo passes as `0`, left out); Odoo's page wrappers are see-through,
    /// their content is read and the external and internal layouts become the page header and
    /// footer; any other is reported and its content read.
    fn call(&mut self, call: &str, node: Node<'a, 'i>, scope: &Scope, depth: usize) -> bool {
        if let Some((name, template)) = self.local_template(call) {
            if self.calls.contains(&name) || self.calls.len() >= 8 {
                self.warn(format!("`t-call=\"{call}\"` calls itself: left out"));
                return true;
            }
            self.enter_call(node, name, scope);
            self.children(template, scope, depth);
            self.leave_call(node);
            return true;
        }
        match call {
            "web.html_container" | "web.basic_layout" => {}
            "web.external_layout" | "web.internal_layout" => {
                let layout = if call == "web.external_layout" { Layout::External } else { Layout::Internal };
                // The company is the document's; a document template read alone names it `o`.
                let doc = scope.doc_var.clone().unwrap_or_else(|| "o".to_owned());
                if self.letterhead.as_ref().is_some_and(|(l, _)| *l != layout) {
                    self.warn(format!("`t-call=\"{call}\"`: the template already calls another page layout; the first one is kept"));
                } else {
                    self.letterhead.get_or_insert((layout, doc));
                }
                if layout == Layout::External {
                    self.layout_blocks(node, scope, depth);
                }
            }
            other => self.warn(format!("`t-call=\"{other}\"`: the called template isn't in this file; the content passed to it is read")),
        }
        false
    }

    /// The template of this file a `t-call` names (by its full or short name); a template that
    /// only makes another one primary under its own name (`inherit_id`, no content) is that one.
    fn local_template(&self, call: &str) -> Option<(String, Node<'a, 'i>)> {
        let find = |wanted: &str| {
            let short = wanted.rsplit('.').next().unwrap_or(wanted);
            self.library.iter().find(|(n, _)| n == wanted || n.rsplit('.').next() == Some(short)).map(|(n, node)| (n.clone(), *node))
        };
        let mut found = find(call)?;
        for _ in 0..4 {
            match found.1.attribute("inherit_id") {
                Some(base) if !found.1.children().any(|c| c.is_element()) => found = find(base)?,
                _ => break,
            }
        }
        Some(found)
    }

    /// Starts reading a called template: the call's attributes are names in it
    /// (`tax_totals="o.tax_totals"`), as QWeb passes them.
    fn enter_call(&mut self, node: Node<'a, 'i>, name: String, scope: &Scope) {
        for a in node.attributes().filter(|a| a.namespace().is_none() && !a.name().starts_with("t-")) {
            self.set(a.name(), Some(a.value()), scope);
        }
        self.calls.push(name);
    }

    /// Ends reading a called template: the names its call passed are gone.
    fn leave_call(&mut self, node: Node<'a, 'i>) {
        self.calls.pop();
        for a in node.attributes().filter(|a| a.namespace().is_none() && !a.name().starts_with("t-")) {
            self.forget(a.name());
        }
    }

    /// A table's rows and the `t-set`s between them, in order, with the rows of the templates its
    /// `t-call`s between rows name (an invoice's tax totals), each row with the calls it is in.
    fn table_items(&self, table: Node<'a, 'i>) -> Vec<TableItem<'a, 'i>> {
        let mut out = Vec::new();
        self.rows_of(table, table, &mut Vec::new(), &mut out);
        out
    }

    fn rows_of(&self, root: Node<'a, 'i>, table: Node<'a, 'i>, chain: &mut Vec<Node<'a, 'i>>, out: &mut Vec<TableItem<'a, 'i>>) {
        // Inside the table, its own rows; inside a called template, those of no table of its own.
        let mine = |n: &Node<'a, 'i>, tags: &[&str]| {
            let owner = n.ancestors().skip(1).find(|a| tags.contains(&a.tag_name().name()));
            if root == table { owner == Some(table) } else { owner.is_none() }
        };
        for n in root.descendants().skip(1) {
            if n.tag_name().name() == "tr" && mine(&n, &["table"]) || n.has_attribute("t-set") && mine(&n, &["table", "tr"]) {
                out.push(TableItem::Row(n, chain.clone()));
            } else if let Some(call) = n.attribute("t-call").filter(|_| mine(&n, &["table", "tr"]) && chain.len() < 8)
                && let Some((name, template)) = self.local_template(call)
                && !self.calls.contains(&name)
            {
                out.push(TableItem::Enter(n, name));
                chain.push(n);
                self.rows_of(template, table, chain, out);
                chain.pop();
                out.push(TableItem::Leave(n));
            }
        }
    }

    fn foreach(&mut self, node: Node<'a, 'i>, each: &str, mut scope: Scope, depth: usize) {
        let var = node.attribute("t-as").unwrap_or("item").to_owned();
        let each = &self.resolve(&expr::keys(each.trim()));
        if crate::odoo::single_loop(each) {
            self.forget(&var);
            self.aliases.push((var, each.clone()));
            self.html(node, &scope, depth);
            return;
        }
        let is_docs = self.doc_loop.is_none() && scope.doc_var.is_none() && each == "docs";
        let is_lines = !self.line_loop
            && scope.line_var.is_none()
            && scope.doc_var.as_deref().is_some_and(|d| each.strip_prefix(d).is_some_and(|rest| rest.starts_with('.')))
            && expr::path(each).is_some();
        if is_docs {
            self.flush(&scope);
            self.doc_loop = Some(var.clone());
            scope.doc_var = Some(var);
            self.children(node, &scope, depth);
            self.flush(&scope);
        } else if is_lines && self.cell.is_none() {
            self.flush(&scope);
            self.line_loop = true;
            self.lines = Some((var.clone(), each.clone()));
            self.target = Target::Detail;
            scope.line_var = Some(var);
            if node.tag_name().name() == "tr" {
                self.row(node, &scope, depth, &[]);
            } else {
                self.children(node, &scope, depth);
                self.flush(&scope);
            }
            self.target = Target::Footer;
        } else if self.open_nested(node, &var, each, &scope, None) {
            scope.item_var = Some(var);
            if node.tag_name().name() == "tr" {
                self.row(node, &scope, depth, &[]);
            } else {
                self.children(node, &scope, depth);
            }
            self.close_nested();
        } else {
            self.warn(format!("loop `{each}` isn't repeated: its content is shown once"));
            self.children(node, &scope, depth);
        }
    }

    fn children(&mut self, node: Node<'a, 'i>, scope: &Scope, depth: usize) {
        for child in node.children() {
            self.walk(child, scope, depth + 1);
        }
    }

    fn html(&mut self, node: Node<'a, 'i>, scope: &Scope, depth: usize) {
        match node.tag_name().name() {
            "table" if self.cell.is_none() => {
                self.flush(scope);
                self.table(node, scope, depth);
            }
            "br" => self.flush(scope),
            "hr" => {
                self.flush(scope);
                let y = self.bottom() + 1.0;
                let line = Content::Line { thickness_mm: 0.2, color: "#000000".into() };
                let (x, width) = self.area();
                self.push(element(x, y, width, 0.0, Style::default().text(), line, scope));
                self.set_bottom(y + 1.0);
            }
            "img" => self.image(node, scope),
            "script" | "style" | "head" | "title" => {}
            "div" if self.grid(node, scope, depth) || self.float_right(node, scope, depth) => {}
            tag if is_block(tag) => {
                self.flush(scope);
                self.children(node, scope, depth);
                self.flush(scope);
                if matches!(tag, "h1" | "h2" | "h3" | "h4" | "p") {
                    let after = self.bottom() + 1.5;
                    self.set_bottom(after);
                }
            }
            _ => self.children(node, scope, depth),
        }
    }

    /// A Bootstrap grid row (`<div class="row">` whose children are `col…` divs) laid out side by
    /// side: each column from the row's top, as wide as its class says (`col-6`, `offset-1`,
    /// `ms-auto`; plain `col`s share what is left), the row as tall as its tallest column. `false`
    /// when it isn't one, or holds a loop or a call (bands of their own), and is read top down.
    fn grid(&mut self, node: Node<'a, 'i>, scope: &Scope, depth: usize) -> bool {
        if self.cell.is_some() || !print_classes(node).split_whitespace().any(|c| c == "row") {
            return false;
        }
        let cols: Vec<Node<'a, 'i>> = node.children().filter(|n| n.is_element()).collect();
        let specs: Option<Vec<ColumnSpec>> = cols.iter().map(|c| column_spec(*c)).collect();
        let Some(specs) = specs.filter(|s| !s.is_empty()) else { return false };
        if node.descendants().any(|n| n.has_attribute("t-foreach") || n.has_attribute("t-call")) {
            return false;
        }
        self.flush(scope);
        let (x0, width) = self.area();
        let unit = width / 12.0;
        let fixed: f64 = specs.iter().map(|s| f64::from(s.span.unwrap_or(0) + s.offset)).sum();
        let autos = specs.iter().filter(|s| s.span.is_none()).count();
        let auto = if autos == 0 { 0.0 } else { ((12.0 - fixed).max(0.0) / autos as f64).max(1.0) };
        let widths: Vec<f64> = specs.iter().map(|s| s.span.map_or(auto, f64::from) * unit).collect();
        let top = self.bottom();
        let outer = self.frame;
        let (mut x, mut bottom) = (x0, top);
        for (i, (col, spec)) in cols.iter().zip(&specs).enumerate() {
            x += f64::from(spec.offset) * unit;
            if spec.push_right {
                let rest: f64 = widths.iter().skip(i).sum();
                x = x.max(x0 + width - rest);
            }
            let w = widths.get(i).copied().unwrap_or(0.0).min((x0 + width - x).max(0.0));
            if w < 1.0 {
                self.warn("a grid column beyond the row's width is left out");
                continue;
            }
            bottom = bottom.max(self.in_frame((*col, true), (x, w), top, scope, depth));
            x += w;
        }
        self.frame = outer;
        self.set_bottom(bottom);
        true
    }

    /// A block floated to the right (`float-end`, an invoice's totals): read in the right half.
    /// Not when it holds the lines' loop (its rows are the detail band).
    fn float_right(&mut self, node: Node<'a, 'i>, scope: &Scope, depth: usize) -> bool {
        let floats = print_classes(node).split_whitespace().any(|c| c == "float-end" || c == "float-right");
        if !floats || self.cell.is_some() || (!self.line_loop && node.descendants().any(|n| n.has_attribute("t-foreach"))) {
            return false;
        }
        self.flush(scope);
        let (x, width) = self.area();
        let top = self.bottom();
        let bottom = self.in_frame((node, false), (x + width / 2.0, width / 2.0), top, scope, depth);
        self.set_bottom(bottom);
        true
    }

    fn text(&mut self, raw: &str, scope: &Scope) {
        let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() {
            if !raw.is_empty() {
                self.space();
            }
            return;
        }
        let lead = raw.starts_with(char::is_whitespace);
        let trail = raw.ends_with(char::is_whitespace);
        let text = format!("{}{collapsed}{}", if lead { " " } else { "" }, if trail { " " } else { "" });
        self.piece(Piece::Text(text, scope.style.text()), scope);
    }

    /// A space between pieces, kept on the text before it.
    fn space(&mut self) {
        let last = match self.cell.as_mut() {
            Some(cell) => cell.last_mut().map(|(p, _)| p),
            None => self.pending.last_mut().map(|(p, _)| p),
        };
        if let Some(Piece::Text(t, _)) = last
            && !t.ends_with(' ')
        {
            t.push(' ');
        }
    }

    fn piece(&mut self, piece: Piece, scope: &Scope) {
        match self.cell.as_mut() {
            Some(cell) => cell.push((piece, scope.condition.clone())),
            None => self.pending.push((piece, scope.condition.clone())),
        }
    }

    fn value(&mut self, value: &str, node: Node<'a, 'i>, scope: &Scope) {
        let style = scope.style.text();
        let (path, helper) = match expr::path(value) {
            Some(p) => (Some(p.to_owned()), None),
            None => match expr::formatted(value) {
                Some((p, numeric)) => (Some(p.to_owned()), Some(numeric)),
                None => (self.join(value, scope), None),
            },
        };
        let path = path.map(|p| self.resolve(&p)).filter(|p| self.known(p, scope));
        if let Some(p) = &path
            && node.attribute("t-options").is_some_and(|o| o.contains("contact"))
        {
            self.contact(p, node, scope);
            return;
        }
        match path {
            Some(p) => {
                let monetary = helper == Some(true)
                    || node.attribute("t-options").is_some_and(|o| o.contains("monetary"))
                    || node.attribute("t-options-widget").is_some_and(|o| o.contains("monetary"));
                let format = self.field(&p, scope).or(monetary.then_some(Format::Number { decimals: 2, grouping: true }));
                self.piece(Piece::Field(p, style, format), scope);
            }
            None => match self
                .calculation(value, scope)
                .or_else(|| self.section_total(value, scope).map(|f| (f, Some(Format::Number { decimals: 2, grouping: true }))))
            {
                Some((formula, format)) => self.piece(Piece::Formula(formula, style, format), scope),
                None => {
                    self.warn(format!("expression `{}` isn't converted: shown as written", collapse(value)));
                    self.piece(Piece::Raw(collapse(value), style), scope);
                }
            },
        }
    }

    /// A partner shown with Odoo's `contact` widget (`t-options='{"widget": "contact", "fields":
    /// ["address", "name"]}'`): a line per part it lists, the name (with its company's before it,
    /// for a contact of a company), then the street, the second street, the postcode and city and
    /// the country, then the phone and email. A part with nothing to show takes no room (the band
    /// shrinks).
    fn contact(&mut self, partner: &str, node: Node<'a, 'i>, scope: &Scope) {
        let options = node.attribute("t-options").unwrap_or("");
        let listed = options.contains("fields");
        let wants = |part: &str| {
            if listed {
                options.contains(&format!("'{part}'")) || options.contains(&format!("\"{part}\""))
            } else {
                matches!(part, "name" | "address")
            }
        };
        let mut lines: Vec<String> = Vec::new();
        let f = |leaf: &str| format!("[{partner}.{leaf}]");
        if wants("name") {
            let (name, company) = (f("name"), f("parent_id.name"));
            lines.push(format!("IF(OR(ISBLANK({company}), {company} = {name}), {name}, {company} & \", \" & {name})"));
        }
        if wants("address") {
            lines.push(f("street"));
            lines.push(f("street2"));
            lines.push(format!("TRIM({} & \" \" & {})", f("zip"), f("city")));
            lines.push(f("country_id.name"));
        }
        for part in ["phone", "mobile", "email", "website", "vat"] {
            if wants(part) {
                lines.push(f(part));
            }
        }
        self.flush(scope);
        let style = scope.style.text();
        for formula in lines {
            for field in formula.split('[').skip(1).filter_map(|x| x.split(']').next()) {
                self.field(field, scope);
            }
            let piece = match formula.strip_prefix('[').and_then(|r| r.strip_suffix(']')).filter(|r| !r.contains('[')) {
                Some(field) => Piece::Field(field.to_owned(), style, None),
                None => Piece::Formula(formula, style, None),
            };
            self.piece(piece, scope);
            self.flush(scope);
        }
    }

    /// A calculation over fields (`line.price_unit * 1.2`) as a formula, with two decimals when
    /// it reads an amount.
    fn calculation(&mut self, value: &str, scope: &Scope) -> Option<(String, Option<Format>)> {
        let formula = self.read_fields(&expr::arithmetic(&collapse(value))?, scope)?;
        let mut amount = false;
        for field in formula.split('[').skip(1).filter_map(|f| f.split(']').next()) {
            amount |= self.field(field, scope).is_some();
        }
        let compares = [" = ", " <> ", " < ", " > ", " <= ", " >= "].iter().any(|c| formula.contains(c));
        Some((formula, (amount && !compares).then_some(Format::Number { decimals: 2, grouping: true })))
    }

    /// Records' values joined into a text (`', '.join(line.tax_ids.mapped('name'))`): the column
    /// that holds them, which the query fills (`string_agg`), said once per column.
    fn join(&mut self, value: &str, scope: &Scope) -> Option<String> {
        let (column, separator) = expr::joined(value)?;
        let column = self.resolve(&column);
        if !self.known(&column, scope) {
            return None;
        }
        if !self.joined.iter().any(|(j, _)| *j == column) {
            self.warn(format!("`{column}` holds the values of the records joined with \"{separator}\": the query joins them (`string_agg`)"));
            self.joined.push((column.clone(), separator));
        }
        Some(column)
    }

    /// A section's total on a line of the detail (`section_subtotal`,
    /// `line._get_section_totals(…)`, which Odoo computes in Python), by its name or the Python
    /// a `t-set` name stands for: the sum of the lines'
    /// `price_subtotal` over the rows the band summarises, which is the section once the lines
    /// are grouped by section (see `sections`).
    fn section_total(&mut self, value: &str, scope: &Scope) -> Option<String> {
        let named = self.unread.iter().rev().find(|(n, _)| n == value.trim()).map(|(_, v)| v.as_str());
        let says = |text: &str| {
            let lower = text.to_ascii_lowercase();
            lower.contains("section") && lower.contains("total")
        };
        if !(says(value) || named.is_some_and(says)) || self.target != Target::Detail {
            return None;
        }
        let field = format!("{}.price_subtotal", scope.line_var.as_deref()?);
        self.field(&field, scope);
        self.warn(format!("`{}` is read as the sum of `{field}` over the section's lines", collapse(value)));
        Some(format!("SUM([{field}])"))
    }

    /// Whether a path reads a field of the documents or their lines: it starts with a loop's
    /// variable (any variable when the template has no document loop, as a document template
    /// read alone). Other names are the report's variables (`company`, `is_pro_forma`…).
    fn known(&self, path: &str, scope: &Scope) -> bool {
        let root = path.split('.').next().unwrap_or(path);
        if matches!(root, "True" | "False" | "None") {
            return false;
        }
        match (&scope.doc_var, &scope.line_var) {
            (None, None) => self.doc_loop.is_none(),
            (doc, line) => doc.as_deref() == Some(root) || line.as_deref() == Some(root) || scope.item_var.as_deref() == Some(root),
        }
    }

    fn image(&mut self, node: Node<'a, 'i>, scope: &Scope) {
        let src = node.attribute("t-att-src").or_else(|| node.attribute("t-attf-src"));
        let Some(field) = src.and_then(expr::image_field) else {
            self.warn("a picture is left out: only `t-att-src=\"image_data_uri(field)\"` is read");
            return;
        };
        if self.cell.is_some() {
            self.warn("a picture in a table cell is left out");
            return;
        }
        let field = &self.resolve(field);
        if !self.known(field, scope) {
            self.warn(format!("a picture of `{field}` is left out: it isn't a field of the documents or their lines"));
            return;
        }
        self.field(field, scope);
        let height = style_mm(node, "height").or_else(|| style_mm(node, "max-height")).unwrap_or(20.0).clamp(2.0, 200.0);
        let (x, room) = self.area();
        let width = style_mm(node, "width").or_else(|| style_mm(node, "max-width")).unwrap_or(height * 2.0).clamp(2.0, room.max(2.0));
        self.flush(scope);
        let y = self.bottom();
        let picture = Content::Image { data: None, field: Some(field.to_owned()), fit: ImageFit::Contain };
        self.push(element(x, y, width, height, scope.style.text(), picture, scope));
    }

    /// Lays the pending pieces out as one line at the bottom of the current band (or grid
    /// column). Pieces shown under different conditions (`<span t-if>Invoice</span><span
    /// t-elif>Credit Note</span> <span t-field="o.name"/>`) are one formula, each piece under its
    /// own condition, so the line reads as Odoo prints it.
    pub(crate) fn flush(&mut self, scope: &Scope) {
        if self.cell.is_some() {
            self.space();
            return;
        }
        let mut pending = std::mem::take(&mut self.pending);
        trim(&mut pending);
        let Some(first) = pending.first().map(|(_, c)| c.clone()) else { return };
        let (x, width) = self.area();
        let y = self.bottom();
        if pending.iter().all(|(_, c)| *c == first) {
            let pieces: Vec<Piece> = pending.into_iter().map(|(p, _)| p).collect();
            let height = line_height(&pieces);
            let scope = Scope { condition: first, ..scope.clone() };
            for el in line(pieces, x, width, y, height, &scope) {
                self.push(el);
            }
            return;
        }
        let outer = scope.condition.clone();
        let piece = merged(pending, outer.as_ref());
        let height = line_height(std::slice::from_ref(&piece));
        for el in line(vec![piece], x, width, y, height, &Scope { condition: outer, ..scope.clone() }) {
            self.push(el);
        }
    }

    fn table(&mut self, node: Node<'a, 'i>, scope: &Scope, depth: usize) {
        let items = self.table_items(node);
        let rows: Vec<Node<'a, 'i>> = items.iter().filter_map(|i| if let TableItem::Row(r, _) = i { Some(*r) } else { None }).collect();
        // The header row decides the columns when there is one (a `colspan="99"` row spans them
        // all), else the widest row.
        // Cells of a `t-elif` / `t-else` branch stand in for the `t-if` branch's: not counted.
        let alternative = |r: Node<'a, 'i>, c: Node<'a, 'i>| branch_of(r, c).is_some_and(|b| !b.has_attribute("t-if"));
        let width = |r: &Node<'a, 'i>| cells(*r).iter().filter(|c| !alternative(*r, **c)).map(|c| span(*c)).sum::<usize>();
        let trs: Vec<Node<'a, 'i>> = rows.iter().copied().filter(|r| r.tag_name().name() == "tr").collect();
        let head = trs.iter().find(|r| r.ancestors().any(|a| a.tag_name().name() == "thead")).map(width);
        let columns = head.filter(|n| *n > 0).or_else(|| trs.iter().map(width).max()).unwrap_or(0);
        if columns == 0 {
            return;
        }
        let widths = vec![self.area().1 / columns as f64; columns];
        for item in items {
            let (tr, calls) = match item {
                TableItem::Row(tr, calls) => (tr, calls),
                TableItem::Enter(call, name) => {
                    self.enter_call(call, name, scope);
                    continue;
                }
                TableItem::Leave(call) => {
                    self.leave_call(call);
                    continue;
                }
            };
            self.line = node.document().text_pos_at(tr.range().start).row;
            if let Some(var) = tr.attribute("t-set") {
                let set_scope = self.loop_scope(tr, node, scope);
                self.set(var, tr.attribute("t-value"), &set_scope);
                continue;
            }
            let mut row_scope = scope.clone();
            // A `t-foreach` or `t-if` on the row or on a `t` wrapping it.
            let mut wrappers: Vec<Node<'a, 'i>> = tr.ancestors().skip(1).take_while(|a| *a != node && a.tag_name().name() == "t").collect();
            wrappers.reverse();
            // The calls it is in come first (their `t-if`: `<t t-if="o.tax_totals" t-call=…>`).
            let mut wrappers: Vec<Node<'a, 'i>> = calls.into_iter().chain(wrappers).collect();
            wrappers.push(tr);
            let mut skipped = false;
            let mut loops = Vec::new();
            for w in &wrappers {
                skipped |= !self.conditions(*w, &mut row_scope);
                // Inside the loops, conditions read their variables: the detail loop's is the
                // line's, another's an item's.
                if let Some(each) = w.attribute("t-foreach") {
                    let var = w.attribute("t-as").unwrap_or("item").to_owned();
                    let resolved = self.resolve(&expr::keys(each.trim()));
                    // A loop over a value that holds one item (an invoice's tax subtotals) is read
                    // once, its variable naming that item.
                    if crate::odoo::single_loop(&resolved) {
                        self.forget(&var);
                        self.aliases.push((var, resolved));
                        continue;
                    }
                    let detail = self.detail_loop == Some(w.id()) || (!self.line_loop && row_scope.doc_var.is_some());
                    if loops.is_empty() && detail {
                        row_scope.line_var = Some(var);
                    } else if Self::nestable(&resolved, &row_scope) {
                        row_scope.item_var = Some(var);
                    }
                    loops.push((*w, each));
                }
            }
            // A nested loop of this table ends at the first row outside it.
            if self.nested.as_ref().is_some_and(|n| n.table == Some(node.id()) && !loops.iter().any(|(w, _)| w.id() == n.node)) {
                self.close_nested();
                if loops.is_empty() && self.target == Target::Detail {
                    self.target = Target::Footer;
                }
            }
            if skipped {
                continue;
            }
            match loops.as_slice() {
                [(w, each), inner @ ..] => {
                    let header = tr.ancestors().any(|a| a.tag_name().name() == "thead");
                    if header {
                        self.warn(format!("loop `{each}` in a table header isn't repeated"));
                        self.row(tr, &row_scope, depth, &widths);
                    } else {
                        let mut s = row_scope.clone();
                        s.style = s.style.inside(*w);
                        if let Some((deeper, each)) = inner.get(1) {
                            self.line = node.document().text_pos_at(deeper.range().start).row;
                            self.warn(format!("loop `{}` inside a nested loop isn't repeated", collapse(each)));
                        }
                        self.table_loop(tr, (*w, each), inner.first().copied(), s, depth, (&widths, node.id()));
                    }
                }
                [] => {
                    let header = tr.ancestors().any(|a| a.tag_name().name() == "thead");
                    self.row(tr, &row_scope, depth, &widths);
                    if header {
                        let y = self.bottom();
                        let (x, width) = self.area();
                        let rule = Content::Line { thickness_mm: 0.3, color: "#000000".into() };
                        self.push(element(x, y, width, 0.0, Style::default().text(), rule, &row_scope));
                    }
                }
            }
        }
        if self.nested.as_ref().is_some_and(|n| n.table == Some(node.id())) {
            self.close_nested();
            if self.target == Target::Detail {
                self.target = Target::Footer;
            }
        }
        let after = self.bottom() + 2.0;
        self.set_bottom(after);
    }

    /// A table row repeated by a loop: the detail band when it is the first loop of a document's
    /// table (whatever it loops over: the report's data gives the lines), each of its rows a part
    /// of the detail band; another loop, or one inside the detail loop (`inner`), a nested loop.
    fn table_loop(
        &mut self,
        tr: Node<'a, 'i>,
        (w, each): (Node<'a, 'i>, &str),
        inner: Option<(Node<'a, 'i>, &str)>,
        scope: Scope,
        depth: usize,
        (widths, table): (&[f64], roxmltree::NodeId),
    ) {
        let each = self.resolve(&expr::keys(each.trim()));
        let same = self.detail_loop == Some(w.id());
        let first = !self.line_loop && scope.doc_var.is_some();
        if !same && !first {
            self.nested_row(tr, (w, &each), scope, depth, (widths, table));
            return;
        }
        if first {
            let on_doc = scope.doc_var.as_deref().is_some_and(|d| each.strip_prefix(d).is_some_and(|rest| rest.starts_with('.')));
            if !on_doc || expr::path(&each).is_none() {
                self.warn(format!("loop `{}` gives the detail rows: the report's data has one row per line", collapse(&each)));
            }
            self.line_loop = true;
            self.lines = Some((w.attribute("t-as").unwrap_or("item").to_owned(), each.clone()));
            self.detail_loop = Some(w.id());
        }
        if self.nested.as_ref().is_some_and(|n| n.table == Some(table) && inner.is_none_or(|(i, _)| i.id() != n.node)) {
            self.close_nested();
        }
        if self.target != Target::Nested {
            self.target = Target::Detail;
        }
        match inner {
            Some((i, e)) => {
                let e = self.resolve(&expr::keys(e.trim()));
                self.nested_row(tr, (i, &e), scope, depth, (widths, table));
            }
            None => self.row(tr, &scope, depth, widths),
        }
        if self.target == Target::Detail {
            self.target = Target::Footer;
        }
    }

    /// A row of a nested loop: into that loop's band, opened at its first row (else shown once).
    fn nested_row(
        &mut self,
        tr: Node<'a, 'i>,
        (w, each): (Node<'a, 'i>, &str),
        scope: Scope,
        depth: usize,
        (widths, table): (&[f64], roxmltree::NodeId),
    ) {
        if !self.nested.as_ref().is_some_and(|n| n.node == w.id()) {
            let var = w.attribute("t-as").unwrap_or("item").to_owned();
            if !self.open_nested(w, &var, each, &scope, Some(table)) {
                self.warn(format!("loop `{}` isn't repeated: its row is shown once", collapse(each)));
                self.row(tr, &scope, depth, widths);
                return;
            }
        }
        self.row(tr, &scope, depth, widths);
    }

    /// The scope of a node inside a table's loops (a `t-set` between rows): the detail loop's
    /// variable is the line's, another's an item's.
    fn loop_scope(&self, node: Node<'a, 'i>, table: Node<'a, 'i>, scope: &Scope) -> Scope {
        let mut out = scope.clone();
        let mut wrappers: Vec<Node<'a, 'i>> = node.ancestors().skip(1).take_while(|a| *a != table && a.tag_name().name() == "t").collect();
        wrappers.reverse();
        let mut first = true;
        for w in wrappers.iter().filter(|w| w.has_attribute("t-foreach")) {
            let var = w.attribute("t-as").unwrap_or("item").to_owned();
            let detail = self.detail_loop == Some(w.id()) || (!self.line_loop && out.doc_var.is_some());
            if first && detail {
                out.line_var = Some(var);
            } else {
                out.item_var = Some(var);
            }
            first = false;
        }
        out
    }

    /// One table row at the bottom of the current band: each cell's pieces in its column. Cells
    /// under `t-if` / `t-elif` / `t-else` (on themselves or on a `t` around them) are
    /// alternatives: each branch starts at the column the first one started at. A cell whose
    /// `colspan` is computed spans the columns left, less one for each cell after it in its
    /// branch.
    fn row(&mut self, tr: Node<'a, 'i>, scope: &Scope, depth: usize, widths: &[f64]) {
        // The row's own `t-set`s first: its cells read them.
        for set in tr.descendants().filter(|n| n.has_attribute("t-set") && n.ancestors().skip(1).find(|a| is_cell(*a) || *a == tr) == Some(tr)) {
            if let Some(var) = set.attribute("t-set") {
                self.line = set.document().text_pos_at(set.range().start).row;
                self.set(var, set.attribute("t-value"), scope);
            }
        }
        let row_cells = cells(tr);
        let (left, room) = self.area();
        let widths = if widths.is_empty() { vec![room / row_cells.len().max(1) as f64; row_cells.len().max(1)] } else { widths.to_vec() };
        let branches: Vec<Option<Node<'a, 'i>>> = row_cells.iter().map(|c| branch_of(tr, *c)).collect();
        let y = self.bottom();
        let (mut x, mut column) = (left, 0);
        let mut chain: (Option<roxmltree::NodeId>, f64, usize) = (None, 0.0, 0);
        let mut laid = Vec::new();
        for (i, cell) in row_cells.iter().copied().enumerate() {
            let branch = branches.get(i).copied().flatten();
            match branch {
                Some(b) if chain.0 != Some(b.id()) => {
                    if b.has_attribute("t-if") {
                        (chain.1, chain.2) = (x, column);
                    } else {
                        (x, column) = (chain.1, chain.2);
                    }
                    chain.0 = Some(b.id());
                }
                Some(_) => {}
                None => chain.0 = None,
            }
            let n = if cell.has_attribute("t-att-colspan") && !cell.has_attribute("colspan") {
                let after = branches.iter().skip(i + 1).filter(|b| b.map(|n| n.id()) == branch.map(|n| n.id())).count();
                widths.len().saturating_sub(column + after).max(1)
            } else {
                span(cell)
            };
            let width: f64 = widths.iter().skip(column).take(n).sum();
            column += n;
            if width <= 0.0 {
                self.warn("a cell beyond the table header's columns is left out");
                continue;
            }
            let mut cell_scope = scope.clone();
            let mut shown = true;
            for w in between(tr, cell).into_iter().chain(std::iter::once(cell)) {
                shown &= self.conditions(w, &mut cell_scope);
            }
            cell_scope.style = scope.style.inside(cell);
            if !shown {
                x += width;
                continue;
            }
            self.cell = Some(Vec::new());
            // A cell may be a field itself (`<td t-field="line.name"/>`).
            match ["t-field", "t-out", "t-esc", "t-raw"].iter().find_map(|a| cell.attribute(*a)) {
                Some(value) => self.value(&expr::keys(value), cell, &cell_scope),
                None => self.children(cell, &cell_scope, depth + 1),
            }
            for (pieces, condition) in variants(self.cell.take().unwrap_or_default(), &cell_scope.condition) {
                laid.push((pieces, x, width, Scope { condition, ..cell_scope.clone() }));
            }
            x += width;
        }
        let height = laid.iter().map(|(p, ..)| line_height(p)).fold(0.0, f64::max).max(5.0);
        for (pieces, x, width, cell_scope) in laid {
            for el in line(pieces, x + 0.5, (width - 1.0).max(1.0), y + 0.5, height, &cell_scope) {
                self.push(el);
            }
        }
        let bottom = y + height + 1.0;
        self.set_bottom(bottom);
    }
}

/// A cell's pieces as what it shows: one line under the cell's condition, or, when every piece
/// shows under a condition of its own (`<span t-if="…tax_excluded"/><span t-if="…tax_included"/>`),
/// one line per condition, laid over each other in the cell as the alternatives they are.
fn variants(pieces: Vec<(Piece, Option<String>)>, base: &Option<String>) -> Vec<(Vec<Piece>, Option<String>)> {
    let mut runs: Vec<(Vec<Piece>, Option<String>)> = Vec::new();
    for (piece, condition) in pieces {
        match runs.last_mut() {
            Some((run, c)) if *c == condition => run.push(piece),
            _ => runs.push((vec![piece], condition)),
        }
    }
    for (run, _) in &mut runs {
        let mut pairs: Vec<(Piece, ())> = std::mem::take(run).into_iter().map(|p| (p, ())).collect();
        trim(&mut pairs);
        *run = pairs.into_iter().map(|(p, ())| p).collect();
    }
    runs.retain(|(run, _)| !run.is_empty());
    if runs.len() > 1 && runs.iter().all(|(_, c)| c != base) {
        return runs;
    }
    // Pieces of the cell's own and pieces under conditions: one formula.
    let mut all: Vec<(Piece, Option<String>)> = runs.into_iter().flat_map(|(run, c)| run.into_iter().map(move |p| (p, c.clone()))).collect();
    trim(&mut all);
    if all.is_empty() {
        return Vec::new();
    }
    if all.iter().all(|(_, c)| c == base) {
        return vec![(all.into_iter().map(|(p, _)| p).collect(), base.clone())];
    }
    vec![(vec![merged(all, base.as_ref())], base.clone())]
}

/// Pieces shown under different conditions as one formula: a piece under a condition other than
/// `outer` (the line's own) shows only when it holds.
fn merged(pieces: Vec<(Piece, Option<String>)>, outer: Option<&String>) -> Piece {
    let parts: Vec<String> = pieces
        .iter()
        .map(|(p, c)| match c {
            Some(c) if outer != Some(c) => format!("IF({c}, {}, \"\")", formula_part(p)),
            _ => formula_part(p),
        })
        .collect();
    let style = pieces.first().map(|(p, _)| *p.style()).unwrap_or_default();
    Piece::Formula(format!("TRIM({})", parts.join(" & ")), style, None)
}

/// A grid column's width in twelfths (`None` for a plain `col`, which shares what is left), the
/// twelfths left empty before it, and whether `ms-auto` pushes it to the right.
struct ColumnSpec {
    span: Option<u8>,
    offset: u8,
    push_right: bool,
}

/// A grid column's classes (`col`, `col-6`, `col-md-5`, `offset-1`, `ms-auto`); `None` when the
/// element isn't a column.
fn column_spec(node: Node<'_, '_>) -> Option<ColumnSpec> {
    let classes = print_classes(node);
    let twelfths = |c: &str| c.rsplit('-').next().and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=12).contains(n));
    let mut spec: Option<ColumnSpec> = None;
    let (mut offset, mut push_right) = (0, false);
    for c in classes.split_whitespace() {
        if c == "col" || c.starts_with("col-") {
            let span = twelfths(c);
            if spec.as_ref().is_none_or(|s| s.span.is_none()) {
                spec = Some(ColumnSpec { span, offset: 0, push_right: false });
            }
        } else if c.starts_with("offset-") {
            offset = twelfths(c).unwrap_or(0);
        } else if c == "ms-auto" || c == "ml-auto" {
            push_right = true;
        }
    }
    spec.map(|s| ColumnSpec { offset, push_right, ..s })
}

fn is_cell(n: Node<'_, '_>) -> bool {
    matches!(n.tag_name().name(), "td" | "th")
}

/// The elements between a row and one of its cells (`t` wrappers), outermost first.
fn between<'a, 'i>(tr: Node<'a, 'i>, cell: Node<'a, 'i>) -> Vec<Node<'a, 'i>> {
    let mut out: Vec<Node<'a, 'i>> = cell.ancestors().skip(1).take_while(|a| *a != tr).collect();
    out.reverse();
    out
}

/// The outermost element of a cell (a `t` around it, or the cell) that is a branch of a
/// `t-if` / `t-elif` / `t-else` chain.
fn branch_of<'a, 'i>(tr: Node<'a, 'i>, cell: Node<'a, 'i>) -> Option<Node<'a, 'i>> {
    between(tr, cell)
        .into_iter()
        .chain(std::iter::once(cell))
        .find(|n| n.has_attribute("t-if") || n.has_attribute("t-elif") || n.has_attribute("t-else"))
}

/// The classes an element has when printed: its `class`, and those `t-att-class` and
/// `t-attf-class` give where they don't depend on the record. Odoo's templates hide what is for
/// the screen with `'d-none …' if report_type == 'html' else ''`: the PDF branch is taken.
fn print_classes(node: Node<'_, '_>) -> String {
    let mut out = node.attribute("class").unwrap_or("").to_owned();
    if let Some(e) = node.attribute("t-att-class") {
        out.push(' ');
        out.push_str(&printed_branch(e));
    }
    if let Some(f) = node.attribute("t-attf-class") {
        let mut rest = f;
        // `{{ … }}` or the older `#{ … }`, whichever comes first.
        let next = |r: &str| [("{{", "}}"), ("#{", "}")].into_iter().filter_map(|(o, c)| r.find(o).map(|i| (i, c))).min_by_key(|(i, _)| *i);
        while let Some((open, end)) = next(rest) {
            out.push(' ');
            out.push_str(&rest[..open]);
            let body = &rest[open + 2..];
            let (inside, after) = match body.find(end) {
                Some(close) => (&body[..close], &body[close + end.len()..]),
                None => (body, ""),
            };
            out.push(' ');
            out.push_str(&printed_branch(inside));
            rest = after;
        }
        out.push(' ');
        out.push_str(rest);
    }
    out
}

/// The classes a Python expression gives a printed report: a quoted text, or the branch of
/// `A if report_type == 'html' else B` taken outside a web page; anything else gives none.
fn printed_branch(e: &str) -> String {
    let e = collapse(e);
    let unquote = |t: &str| {
        let t = t.trim();
        t.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')).or_else(|| t.strip_prefix('"').and_then(|x| x.strip_suffix('"'))).map(str::to_owned)
    };
    if let Some(text) = unquote(&e).filter(|t| !t.contains(['\'', '"'])) {
        return text;
    }
    let Some((then, rest)) = e.split_once(" if ") else { return String::new() };
    let Some((test, otherwise)) = rest.split_once(" else ") else { return String::new() };
    let test = test.replace('"', "'").replace(' ', "");
    let taken = match test.as_str() {
        "report_type=='html'" => otherwise,
        "report_type!='html'" => then,
        _ => return String::new(),
    };
    unquote(taken).unwrap_or_default()
}

/// Hidden in a printed report: `d-none` with no `d-print-…` class bringing it back.
fn hidden_in_print(node: Node<'_, '_>) -> bool {
    let classes = print_classes(node);
    classes.split_whitespace().any(|c| c == "d-none") && !classes.split_whitespace().any(|c| c.starts_with("d-print-"))
}

/// Adds a column to a list unless it is there.
pub(crate) fn add_field(fields: &mut Vec<Field>, name: &str, level: Level) {
    if !fields.iter().any(|f| f.name.eq_ignore_ascii_case(name)) {
        fields.push(Field { name: name.to_owned(), field_type: guess_type(name), level, example: None });
    }
}

/// Whitespace runs as one space (conditions written over several lines).
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_block(tag: &str) -> bool {
    matches!(
        tag,
        "div"
            | "p"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "section"
            | "header"
            | "footer"
            | "address"
            | "ul"
            | "ol"
            | "li"
            | "main"
            | "article"
            | "blockquote"
    )
}

/// A row's cells, with those inside `t` wrappers (`<t t-if>` around a `td`), less those a
/// printed report hides.
fn cells<'a, 'i>(tr: Node<'a, 'i>) -> Vec<Node<'a, 'i>> {
    tr.descendants()
        .filter(|n| is_cell(*n) && n.ancestors().skip(1).find(|a| a.tag_name().name() == "tr") == Some(tr) && !hidden_in_print(*n))
        .collect()
}

fn span(cell: Node<'_, '_>) -> usize {
    cell.attribute("colspan").and_then(|c| c.trim().parse::<usize>().ok()).unwrap_or(1).clamp(1, 64)
}

/// A length from an element's `style` (`height: 40px`, `width: 3cm`), in millimetres.
fn style_mm(node: Node<'_, '_>, property: &str) -> Option<f64> {
    let style = node.attribute("style")?;
    let value = style.split(';').find_map(|d| d.split_once(':').filter(|(k, _)| k.trim() == property).map(|(_, v)| v.trim()))?;
    let (number, unit): (String, String) = value.chars().partition(|c| c.is_ascii_digit() || *c == '.');
    let n: f64 = number.parse().ok().filter(|n: &f64| n.is_finite())?;
    match unit.trim() {
        "px" => Some(n * 25.4 / 96.0),
        "mm" => Some(n),
        "cm" => Some(n * 10.0),
        "pt" => Some(n * MM_PER_PT),
        _ => None,
    }
}

/// Removes the spaces at both ends of a line.
fn trim<C>(pieces: &mut Vec<(Piece, C)>) {
    if let Some((Piece::Text(t, _), _)) = pieces.first_mut() {
        *t = t.trim_start().to_owned();
    }
    if let Some((Piece::Text(t, _), _)) = pieces.last_mut() {
        *t = t.trim_end().to_owned();
    }
    pieces.retain(|(p, _)| !matches!(p, Piece::Text(t, _) if t.is_empty()));
}

fn line_height(pieces: &[Piece]) -> f64 {
    let size = pieces.iter().map(|p| p.style().size_pt).fold(0.0, f64::max);
    (size * MM_PER_PT * 1.5 * 2.0).ceil() / 2.0
}

/// A line of pieces as elements from `x`, `width` wide. One piece fills the width with its
/// alignment; several on a left-aligned line are placed one after the other (text as wide as it
/// reads, a field as wide as the room left when it is last); otherwise they are joined in one
/// formula, which keeps the alignment.
fn line(pieces: Vec<Piece>, x: f64, width: f64, y: f64, height: f64, scope: &Scope) -> Vec<Element> {
    let align = pieces.first().map_or(Align::Left, |p| p.style().align);
    if let [only] = pieces.as_slice() {
        let (content, format) = content_of(only);
        let mut el = element(x, y, width, height, *only.style(), content, scope);
        el.can_grow = grows(&el.content, format.as_ref());
        el.format = format;
        return vec![el];
    }
    if align != Align::Left {
        let parts: Vec<String> = pieces.iter().map(formula_part).collect();
        let mut style = pieces.first().map(|p| *p.style()).unwrap_or_default();
        style.align = align;
        let content = Content::Formula { expression: parts.join(" & ") };
        let mut el = element(x, y, width, height, style, content, scope);
        el.can_grow = true;
        return vec![el];
    }
    let mut out = Vec::new();
    let mut left = x;
    let count = pieces.len();
    for (i, piece) in pieces.into_iter().enumerate() {
        let room = (x + width - left).max(0.0);
        if room < 1.0 {
            break;
        }
        let wanted = match &piece {
            Piece::Text(t, s) | Piece::Raw(t, s) => t.chars().count() as f64 * s.size_pt * MM_PER_PT * CHAR_EM + 1.0,
            Piece::Field(..) | Piece::Formula(..) if i + 1 == count => room,
            Piece::Field(..) | Piece::Formula(..) => FIELD_MM,
        };
        let w = wanted.min(room);
        let (content, format) = content_of(&piece);
        let mut el = element(left, y, w, height, *piece.style(), content, scope);
        // The value that ends the line takes the lines it needs, as HTML wraps it.
        el.can_grow = i + 1 == count && grows(&el.content, format.as_ref());
        el.format = format;
        out.push(el);
        left += w;
    }
    out
}

/// A field or formula written as text (not a number) wraps onto the lines it needs.
fn grows(content: &Content, format: Option<&Format>) -> bool {
    matches!(content, Content::Field { .. } | Content::Formula { .. }) && format.is_none()
}

fn content_of(piece: &Piece) -> (Content, Option<Format>) {
    match piece {
        Piece::Text(t, _) => (Content::Text { text: t.clone() }, None),
        Piece::Raw(e, _) => (Content::Text { text: format!("{{{e}}}") }, None),
        Piece::Field(f, _, format) => (Content::Field { field: f.clone() }, format.clone()),
        Piece::Formula(e, _, format) => (Content::Formula { expression: e.clone() }, format.clone()),
    }
}

fn formula_part(piece: &Piece) -> String {
    match piece {
        Piece::Text(t, _) => format!("\"{}\"", t.replace('"', "\"\"")),
        Piece::Raw(e, _) => format!("\"{{{}}}\"", e.replace('"', "\"\"")),
        Piece::Field(f, _, Some(Format::Number { decimals, .. })) => format!("TEXT([{f}], \"#,##0.{}\")", "0".repeat(usize::from(*decimals))),
        Piece::Field(f, ..) => format!("[{f}]"),
        Piece::Formula(e, _, Some(Format::Number { decimals, .. })) => format!("TEXT({e}, \"#,##0.{}\")", "0".repeat(usize::from(*decimals))),
        Piece::Formula(e, ..) => e.clone(),
    }
}

fn element(x: f64, y: f64, width: f64, height: f64, style: TextStyle, content: Content, scope: &Scope) -> Element {
    Element {
        x_mm: round(x),
        y_mm: round(y),
        width_mm: round(width),
        height_mm: round(height),
        style,
        content,
        format: None,
        visible_if: scope.condition.clone(),
        color_formula: None,
        background_formula: None,
        can_grow: false,
    }
}

fn round(mm: f64) -> f64 {
    (mm * 10.0).round() / 10.0
}

/// Last part of a field path, lower case.
fn leaf(name: &str) -> String {
    name.rsplit('.').next().unwrap_or(name).to_ascii_lowercase()
}

/// Amounts: shown with two decimals.
fn money(name: &str) -> bool {
    let l = leaf(name);
    ["price", "amount", "total", "subtotal", "tax", "balance", "cost"].iter().any(|w| l.contains(w))
}

/// The type a field's name suggests.
fn guess_type(name: &str) -> FieldType {
    let l = leaf(name);
    if l == "id" || l.ends_with("_count") || l == "sequence" {
        FieldType::Integer
    } else if expr::flag(name) {
        FieldType::Bool
    } else if money(name) || ["qty", "quantity", "discount", "weight", "volume", "rate"].iter().any(|w| l.contains(w)) {
        FieldType::Number
    } else {
        FieldType::Text
    }
}
