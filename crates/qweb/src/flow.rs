//! Lays a QWeb template's HTML out as bands: a top-to-bottom flow of lines, each line the inline
//! text and fields of a block (`p`, `div`, `h2`…), tables as rows of equal columns. The document
//! loop (`t-foreach="docs"`) becomes a group with a page break, the loop over its lines the detail
//! band; what comes before the lines goes in the group header, what follows in its footer.

use reportcraft_core::FieldType;
use reportcraft_core::limits::MAX_ELEMENTS_PER_BAND;
use reportcraft_model::{Align, Band, Content, Element, Format, ImageFit, Severity, TextStyle};
use roxmltree::Node;

use crate::expr;

/// The width laid out on: A4 less two 15 mm margins.
pub(crate) const CONTENT_MM: f64 = 180.0;
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
}

/// A finding while reading, located by line.
pub(crate) struct Finding {
    pub severity: Severity,
    pub line: u32,
    pub message: String,
}

/// A column the template reads, with the type its name suggests (for sample data and formats).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Field {
    pub name: String,
    pub field_type: FieldType,
    /// Belongs to the line loop (varies on each detail row) rather than to the document.
    pub per_line: bool,
}

/// One piece of a line of text.
enum Piece {
    Text(String, TextStyle),
    Field(String, TextStyle, Option<Format>),
    /// An expression with no ReportCraft meaning, shown as written.
    Raw(String, TextStyle),
}

impl Piece {
    fn style(&self) -> &TextStyle {
        match self {
            Piece::Text(_, s) | Piece::Field(_, s, _) | Piece::Raw(_, s) => s,
        }
    }
}

/// The loop variables in scope and the condition elements are shown under.
#[derive(Clone, Default)]
pub(crate) struct Scope {
    pub doc_var: Option<String>,
    pub line_var: Option<String>,
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
        for class in node.attribute("class").unwrap_or("").split_whitespace() {
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
    pub header: Band,
    pub detail: Band,
    pub footer: Band,
    pub target: Target,
    pub fields: Vec<Field>,
    pub findings: Vec<Finding>,
    /// Found the document loop and the line loop.
    pub doc_loop: Option<String>,
    pub line_loop: bool,
    /// The line of text being gathered, with the condition it shows under.
    pending: Vec<Piece>,
    pending_condition: Option<String>,
    /// A table cell gathers its pieces here instead of laying out lines.
    cell: Option<Vec<Piece>>,
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
            detail_loop: None,
            header: Band::default(),
            detail: Band::default(),
            footer: Band::default(),
            target: Target::Header,
            fields: Vec::new(),
            findings: Vec::new(),
            doc_loop: None,
            line_loop: false,
            pending: Vec::new(),
            pending_condition: None,
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
        band.height_mm = band.height_mm.max(element.y_mm + element.height_mm);
        band.elements.push(element);
    }

    fn bottom(&mut self) -> f64 {
        self.band().height_mm
    }

    /// Records a column the template reads; the first use decides whether it is per line.
    fn field(&mut self, name: &str, scope: &Scope) -> Option<Format> {
        let per_line = scope.line_var.as_deref().is_some_and(|v| name.split('.').next() == Some(v));
        let field_type = guess_type(name);
        if !self.fields.iter().any(|f| f.name.eq_ignore_ascii_case(name)) {
            self.fields.push(Field { name: name.to_owned(), field_type, per_line });
        }
        money(name).then_some(Format::Number { decimals: 2, grouping: true })
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
            && self.call(call, &scope, depth)
        {
            return;
        }
        if let Some(var) = node.attribute("t-set") {
            self.set(var, node.attribute("t-value"));
            return;
        }
        if let Some(value) = ["t-field", "t-out", "t-esc", "t-raw"].iter().find_map(|a| node.attribute(*a)) {
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
                    self.warn(format!("condition `{}` isn't converted: its content is always shown", collapse(test)));
                    return true;
                }
            }
        };
        for field in formula.split('[').skip(1).filter_map(|f| f.split(']').next()) {
            self.field(field, scope);
        }
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
        let formula = expr::condition(&expr::keys(&collapse(test)))?;
        let mut out = String::new();
        let mut rest = formula.as_str();
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
    /// remembered and read through; filters and sorts aren't applied, which is reported.
    fn set(&mut self, var: &str, value: Option<&str>) {
        let Some(value) = value.map(|v| expr::keys(&collapse(v))) else {
            self.warn(format!("`t-set=\"{var}\"` with a body is ignored"));
            return;
        };
        let cut = [".filtered(", ".sorted(", ".with_context(", ".sudo(", ".with_company("].iter().filter_map(|m| value.find(m)).min();
        let (base, tail) = cut.map_or((value.as_str(), ""), |i| value.split_at(i));
        let Some(base) = expr::path(base) else {
            self.warn(format!("`t-set=\"{var}\"` is ignored"));
            return;
        };
        let target = self.resolve(base);
        if target == var {
            return;
        }
        if tail.contains(".filtered(") || tail.contains(".sorted(") {
            self.warn(format!("`t-set=\"{var}\"`: filters and sorts aren't applied, `{var}` reads every record of `{target}`"));
        }
        self.aliases.push((var.to_owned(), target));
    }

    /// Follows a `t-call`: a template of the same file is read in place (`true`, the element's own
    /// content is then what Odoo passes as `0`, left out); the suite's page wrappers are see-through
    /// and their content is read; any other is reported and its content read.
    fn call(&mut self, call: &str, scope: &Scope, depth: usize) -> bool {
        let short = call.rsplit('.').next().unwrap_or(call);
        let local = self.library.iter().find(|(n, _)| n == call || n.rsplit('.').next() == Some(short)).map(|(n, node)| (n.clone(), *node));
        if let Some((name, template)) = local {
            if self.calls.contains(&name) || self.calls.len() >= 8 {
                self.warn(format!("`t-call=\"{call}\"` calls itself: left out"));
                return true;
            }
            self.calls.push(name);
            self.children(template, scope, depth);
            self.calls.pop();
            return true;
        }
        match call {
            "web.html_container" | "web.basic_layout" | "web.internal_layout" => {}
            "web.external_layout" => self.warn("the company letterhead of `web.external_layout` (logo, address, footer) isn't reproduced"),
            other => self.warn(format!("`t-call=\"{other}\"`: the called template isn't in this file; the content passed to it is read")),
        }
        false
    }

    fn foreach(&mut self, node: Node<'a, 'i>, each: &str, mut scope: Scope, depth: usize) {
        let var = node.attribute("t-as").unwrap_or("item").to_owned();
        let each = &self.resolve(each.trim());
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
            self.target = Target::Detail;
            scope.line_var = Some(var);
            if node.tag_name().name() == "tr" {
                self.row(node, &scope, depth, &[]);
            } else {
                self.children(node, &scope, depth);
                self.flush(&scope);
            }
            self.target = Target::Footer;
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
                self.push(element(0.0, y, CONTENT_MM, 0.0, Style::default().text(), line, scope));
                self.band().height_mm = y + 1.0;
            }
            "img" => self.image(node, scope),
            "script" | "style" | "head" | "title" => {}
            tag if is_block(tag) => {
                self.flush(scope);
                self.children(node, scope, depth);
                self.flush(scope);
                if matches!(tag, "h1" | "h2" | "h3" | "h4" | "p") {
                    let after = self.bottom() + 1.5;
                    self.band().height_mm = after;
                }
            }
            _ => self.children(node, scope, depth),
        }
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
        let pieces = self.cell.as_mut().unwrap_or(&mut self.pending);
        if let Some(Piece::Text(t, _)) = pieces.last_mut()
            && !t.ends_with(' ')
        {
            t.push(' ');
        }
    }

    fn piece(&mut self, piece: Piece, scope: &Scope) {
        if let Some(cell) = self.cell.as_mut() {
            cell.push(piece);
            return;
        }
        if self.pending.is_empty() {
            self.pending_condition.clone_from(&scope.condition);
        } else if self.pending_condition != scope.condition {
            // A condition that changes inside a line: the line keeps the first one.
            self.flush(scope);
            self.pending_condition.clone_from(&scope.condition);
        }
        self.pending.push(piece);
    }

    fn value(&mut self, value: &str, node: Node<'a, 'i>, scope: &Scope) {
        let style = scope.style.text();
        let (path, helper) = match expr::path(value) {
            Some(p) => (Some(p), None),
            None => expr::formatted(value).map_or((None, None), |(p, numeric)| (Some(p), Some(numeric))),
        };
        let path = path.map(|p| self.resolve(p)).filter(|p| self.known(p, scope));
        match path {
            Some(p) => {
                let monetary = helper == Some(true)
                    || node.attribute("t-options").is_some_and(|o| o.contains("monetary"))
                    || node.attribute("t-options-widget").is_some_and(|o| o.contains("monetary"));
                let format = self.field(&p, scope).or(monetary.then_some(Format::Number { decimals: 2, grouping: true }));
                self.piece(Piece::Field(p, style, format), scope);
            }
            None => {
                self.warn(format!("expression `{}` isn't converted: shown as written", collapse(value)));
                self.piece(Piece::Raw(collapse(value), style), scope);
            }
        }
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
            (doc, line) => doc.as_deref() == Some(root) || line.as_deref() == Some(root),
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
        let width = style_mm(node, "width").or_else(|| style_mm(node, "max-width")).unwrap_or(height * 2.0).clamp(2.0, CONTENT_MM);
        self.flush(scope);
        let y = self.bottom();
        let picture = Content::Image { data: None, field: Some(field.to_owned()), fit: ImageFit::Contain };
        self.push(element(0.0, y, width, height, scope.style.text(), picture, scope));
    }

    /// Lays the pending pieces out as one line at the bottom of the current band.
    pub(crate) fn flush(&mut self, scope: &Scope) {
        if self.cell.is_some() {
            self.space();
            return;
        }
        let mut pieces = std::mem::take(&mut self.pending);
        trim(&mut pieces);
        if pieces.is_empty() {
            return;
        }
        let condition = self.pending_condition.take();
        let height = line_height(&pieces);
        let y = self.bottom();
        let scope = Scope { condition, ..scope.clone() };
        for el in line(pieces, 0.0, CONTENT_MM, y, height, &scope) {
            self.push(el);
        }
    }

    fn table(&mut self, node: Node<'a, 'i>, scope: &Scope, depth: usize) {
        let rows = table_rows(node);
        // The header row decides the columns when there is one (a `colspan="99"` row spans them
        // all), else the widest row.
        let width = |r: &Node<'a, 'i>| cells(*r).iter().map(|c| span(*c)).sum::<usize>();
        let trs: Vec<Node<'a, 'i>> = rows.iter().copied().filter(|r| r.tag_name().name() == "tr").collect();
        let head = trs.iter().find(|r| r.ancestors().any(|a| a.tag_name().name() == "thead")).map(width);
        let columns = head.filter(|n| *n > 0).or_else(|| trs.iter().map(width).max()).unwrap_or(0);
        if columns == 0 {
            return;
        }
        let widths = vec![CONTENT_MM / columns as f64; columns];
        for tr in rows {
            self.line = node.document().text_pos_at(tr.range().start).row;
            if let Some(var) = tr.attribute("t-set") {
                self.set(var, tr.attribute("t-value"));
                continue;
            }
            let mut row_scope = scope.clone();
            // A `t-foreach` or `t-if` on the row or on a `t` wrapping it.
            let mut wrappers: Vec<Node<'a, 'i>> = tr.ancestors().skip(1).take_while(|a| *a != node && a.tag_name().name() == "t").collect();
            wrappers.reverse();
            wrappers.push(tr);
            let mut skipped = false;
            for w in &wrappers {
                skipped |= !self.conditions(*w, &mut row_scope);
                // Inside the detail loop, conditions read the line's variable.
                if w.has_attribute("t-foreach") && (self.detail_loop == Some(w.id()) || (!self.line_loop && row_scope.doc_var.is_some())) {
                    row_scope.line_var = Some(w.attribute("t-as").unwrap_or("item").to_owned());
                }
            }
            if skipped {
                continue;
            }
            let looped = wrappers.iter().find_map(|w| w.attribute("t-foreach").map(|e| (*w, e)));
            match looped {
                Some((w, each)) => {
                    let header = tr.ancestors().any(|a| a.tag_name().name() == "thead");
                    if header {
                        self.warn(format!("loop `{each}` in a table header isn't repeated"));
                        self.row(tr, &row_scope, depth, &widths);
                    } else {
                        let mut s = row_scope.clone();
                        s.style = s.style.inside(w);
                        self.table_loop(tr, w, each, s, depth, &widths);
                    }
                }
                None => {
                    let header = tr.ancestors().any(|a| a.tag_name().name() == "thead");
                    self.row(tr, &row_scope, depth, &widths);
                    if header {
                        let y = self.bottom();
                        let rule = Content::Line { thickness_mm: 0.3, color: "#000000".into() };
                        self.push(element(0.0, y, CONTENT_MM, 0.0, Style::default().text(), rule, &row_scope));
                    }
                }
            }
        }
        let after = self.bottom() + 2.0;
        self.band().height_mm = after;
    }

    /// A table row repeated by a loop: the detail band when it is the first loop of a document's
    /// table (whatever it loops over: the report's data gives the lines), each of its rows a part
    /// of the detail band.
    fn table_loop(&mut self, tr: Node<'a, 'i>, w: Node<'a, 'i>, each: &str, mut scope: Scope, depth: usize, widths: &[f64]) {
        let var = w.attribute("t-as").unwrap_or("item").to_owned();
        let each = self.resolve(each.trim());
        let same = self.detail_loop == Some(w.id());
        let first = !self.line_loop && scope.doc_var.is_some();
        if !same && !first {
            self.warn(format!("loop `{}` isn't repeated: its row is shown once", collapse(&each)));
            self.row(tr, &scope, depth, widths);
            return;
        }
        if first {
            let on_doc = scope.doc_var.as_deref().is_some_and(|d| each.strip_prefix(d).is_some_and(|rest| rest.starts_with('.')));
            if !on_doc || expr::path(&each).is_none() {
                self.warn(format!("loop `{}` gives the detail rows: the report's data has one row per line", collapse(&each)));
            }
            self.line_loop = true;
            self.detail_loop = Some(w.id());
        }
        self.target = Target::Detail;
        scope.line_var = Some(var);
        self.row(tr, &scope, depth, widths);
        self.target = Target::Footer;
    }

    /// One table row at the bottom of the current band: each cell's pieces in its column.
    fn row(&mut self, tr: Node<'a, 'i>, scope: &Scope, depth: usize, widths: &[f64]) {
        let widths = if widths.is_empty() { vec![CONTENT_MM / cells(tr).len().max(1) as f64; cells(tr).len().max(1)] } else { widths.to_vec() };
        let y = self.bottom();
        let mut x = 0.0;
        let mut column = 0;
        let mut laid = Vec::new();
        for cell in cells(tr) {
            let mut cell_scope = scope.clone();
            cell_scope.style = scope.style.inside(cell);
            let n = span(cell);
            let width: f64 = widths.iter().skip(column).take(n).sum();
            column += n;
            if width <= 0.0 {
                self.warn("a cell beyond the table header's columns is left out");
                continue;
            }
            if !self.conditions(cell, &mut cell_scope) {
                x += width;
                continue;
            }
            self.cell = Some(Vec::new());
            // A cell may be a field itself (`<td t-field="line.name"/>`).
            match ["t-field", "t-out", "t-esc", "t-raw"].iter().find_map(|a| cell.attribute(*a)) {
                Some(value) => self.value(&expr::keys(value), cell, &cell_scope),
                None => self.children(cell, &cell_scope, depth + 1),
            }
            let mut pieces = self.cell.take().unwrap_or_default();
            trim(&mut pieces);
            if !pieces.is_empty() {
                laid.push((pieces, x, width, cell_scope));
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
        self.band().height_mm = bottom;
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

/// The rows of a table and the `t-set`s between them, in order, with those of nested tables left
/// to them.
fn table_rows<'a, 'i>(table: Node<'a, 'i>) -> Vec<Node<'a, 'i>> {
    table
        .descendants()
        .filter(|n| {
            (n.tag_name().name() == "tr" || n.has_attribute("t-set")) && n.ancestors().skip(1).find(|a| a.tag_name().name() == "table") == Some(table)
        })
        .collect()
}

/// A row's cells, with those inside `t` wrappers (`<t t-if>` around a `td`).
fn cells<'a, 'i>(tr: Node<'a, 'i>) -> Vec<Node<'a, 'i>> {
    tr.descendants()
        .filter(|n| matches!(n.tag_name().name(), "td" | "th") && n.ancestors().skip(1).find(|a| a.tag_name().name() == "tr") == Some(tr))
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
fn trim(pieces: &mut Vec<Piece>) {
    if let Some(Piece::Text(t, _)) = pieces.first_mut() {
        *t = t.trim_start().to_owned();
    }
    if let Some(Piece::Text(t, _)) = pieces.last_mut() {
        *t = t.trim_end().to_owned();
    }
    pieces.retain(|p| !matches!(p, Piece::Text(t, _) if t.is_empty()));
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
        el.format = format;
        return vec![el];
    }
    if align != Align::Left {
        let parts: Vec<String> = pieces.iter().map(formula_part).collect();
        let mut style = pieces.first().map(|p| *p.style()).unwrap_or_default();
        style.align = align;
        let content = Content::Formula { expression: parts.join(" & ") };
        return vec![element(x, y, width, height, style, content, scope)];
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
            Piece::Field(..) if i + 1 == count => room,
            Piece::Field(..) => FIELD_MM,
        };
        let w = wanted.min(room);
        let (content, format) = content_of(&piece);
        let mut el = element(left, y, w, height, *piece.style(), content, scope);
        el.format = format;
        out.push(el);
        left += w;
    }
    out
}

fn content_of(piece: &Piece) -> (Content, Option<Format>) {
    match piece {
        Piece::Text(t, _) => (Content::Text { text: t.clone() }, None),
        Piece::Raw(e, _) => (Content::Text { text: format!("{{{e}}}") }, None),
        Piece::Field(f, _, format) => (Content::Field { field: f.clone() }, format.clone()),
    }
}

fn formula_part(piece: &Piece) -> String {
    match piece {
        Piece::Text(t, _) => format!("\"{}\"", t.replace('"', "\"\"")),
        Piece::Raw(e, _) => format!("\"{{{}}}\"", e.replace('"', "\"\"")),
        Piece::Field(f, _, Some(Format::Number { decimals, .. })) => format!("TEXT([{f}], \"#,##0.{}\")", "0".repeat(usize::from(*decimals))),
        Piece::Field(f, ..) => format!("[{f}]"),
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
    } else if money(name) || ["qty", "quantity", "discount", "weight", "volume", "rate"].iter().any(|w| l.contains(w)) {
        FieldType::Number
    } else {
        FieldType::Text
    }
}
