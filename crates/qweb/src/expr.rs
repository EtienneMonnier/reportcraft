//! QWeb expressions (Python) that have a ReportCraft meaning: a field path, a truth test or a
//! comparison. Anything else is left to the caller to report.

/// A dotted path such as `o.partner_id.name`: identifiers joined by dots, nothing else. An
/// expression that falls back to a blank (`o.ref or ''`) counts as its path, since a blank field
/// shows nothing either way.
pub(crate) fn path(expr: &str) -> Option<&str> {
    let e = expr.trim();
    let e = ["or ''", "or \"\"", "or False"].iter().find_map(|tail| e.strip_suffix(tail)).map_or(e, str::trim_end);
    let ok = !e.is_empty()
        && e.split('.').all(|part| {
            let mut chars = part.chars();
            chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
    ok.then_some(e)
}

/// A field whose name says it is a yes/no flag (`is_downpayment`, `collapse_prices`,
/// `show_payment_term_details`, `display_in_company_currency`): Python's truth of a flag is its
/// value, not whether it is blank.
pub(crate) fn flag(path: &str) -> bool {
    let leaf = path.rsplit('.').next().unwrap_or(path).to_ascii_lowercase();
    ["is_", "has_", "show_", "hide_", "collapse_", "display_"].iter().any(|p| leaf.starts_with(p))
        && !matches!(leaf.as_str(), "display_name" | "display_type")
}

/// `path` true, as Python reads it: a flag's value, any other field's being there.
fn truthy(p: &str) -> String {
    if flag(p) { format!("[{p}]") } else { format!("NOT(ISBLANK([{p}]))") }
}

/// `path` false, as Python reads it.
fn falsy(p: &str) -> String {
    if flag(p) { format!("NOT([{p}])") } else { format!("ISBLANK([{p}])") }
}

/// Dictionary keys written as fields: `line['label']` and `totals.get('label')` read as
/// `line.label` (Odoo's grouped lines and tax totals are dictionaries).
pub(crate) fn keys(e: &str) -> String {
    let e = &gets(e);
    let mut out = String::with_capacity(e.len());
    let mut rest = e.as_str();
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        // A key follows a name (`line['label']`), never a space (`in ['a']` is a list).
        let follows_name = rest[..open].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == ']' || c == ')');
        let quote = after.chars().next().filter(|c| (*c == '\'' || *c == '"') && follows_name);
        let key = quote.and_then(|q| after[1..].find(q).map(|end| &after[1..=end]));
        let ident = |k: &str| {
            k.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        };
        match key {
            Some(k) if ident(k) && after[k.len() + 1..].starts_with(&format!("{}]", quote.unwrap_or('\''))) => {
                out.push_str(&rest[..open]);
                out.push('.');
                out.push_str(k);
                rest = &after[k.len() + 3..];
            }
            _ => {
                out.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `.get('key')` (one quoted name, no default) as `['key']`.
fn gets(e: &str) -> String {
    let mut out = String::with_capacity(e.len());
    let mut rest = e;
    while let Some(at) = rest.find(".get(") {
        let after = &rest[at + 5..];
        let quote = after.chars().next().filter(|c| *c == '\'' || *c == '"');
        let key = quote.and_then(|q| after[1..].find(q).map(|end| &after[1..=end]));
        match (quote, key) {
            (Some(q), Some(k))
                if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && after[k.len() + 2..].starts_with(')') =>
            {
                out.push_str(&rest[..at]);
                out.push_str(&format!("[{q}{k}{q}]"));
                rest = &after[k.len() + 3..];
            }
            _ => {
                out.push_str(&rest[..at + 5]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A `t-if` condition as a formula, for the shapes reports use most: a field's truth
/// (`o.note`, `not o.note`), a comparison with a number or a quoted text
/// (`o.state == 'posted'`, `line.discount > 0`), membership in a list of literals
/// (`line.display_type in ('line_section', 'line_note')`, `not in`), joined by `and`, `or` and
/// `not`, with parentheses. `None` for anything else.
#[cfg(test)]
pub(crate) fn condition(expr: &str) -> Option<String> {
    condition_with(expr, &|_| None)
}

/// [`condition`], with `named` giving the formula a bare name stands for (a `t-set` that names a
/// condition, such as `is_section = line.display_type == 'line_section'`).
pub(crate) fn condition_with(expr: &str, named: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    cond(expr, named, 0)
}

/// Deepest nesting of `and`, `or`, `not` and parentheses read.
const MAX_NESTING: usize = 32;

fn cond(expr: &str, named: &dyn Fn(&str) -> Option<String>, depth: usize) -> Option<String> {
    if depth > MAX_NESTING {
        return None;
    }
    let e = expr.trim();
    // `a or b and c` reads as `a or (b and c)`, as in Python.
    for (word, function) in [(" or ", "OR"), (" and ", "AND")] {
        let parts = split_top(e, word)?;
        if parts.len() > 1 {
            let parts: Option<Vec<String>> = parts.iter().map(|p| cond(p, named, depth + 1)).collect();
            return parts.map(|p| format!("{function}({})", p.join(", ")));
        }
    }
    if let Some(inner) = parenthesised(e) {
        return cond(inner, named, depth + 1);
    }
    if let Some(rest) = e.strip_prefix("not ").or_else(|| e.strip_prefix("not(").map(|_| &e[3..])) {
        let rest = rest.trim();
        if let Some(f) = named(rest) {
            return Some(format!("NOT({f})"));
        }
        if let Some(p) = path(rest) {
            return Some(falsy(p));
        }
        return cond(rest, named, depth + 1).map(|f| format!("NOT({f})"));
    }
    simple(e, named)
}

/// `text` cut at `word` where it isn't inside quotes or brackets; `None` when the quotes or
/// brackets don't balance.
fn split_top<'t>(text: &'t str, word: &str) -> Option<Vec<&'t str>> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut start = 0;
    let mut chars = text.char_indices();
    while let Some((i, c)) = chars.next() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, '(' | '[' | '{') => depth += 1,
            (None, ')' | ']' | '}') => depth = depth.checked_sub(1)?,
            (None, _) if depth == 0 && text[i..].starts_with(word) => {
                parts.push(&text[start..i]);
                start = i + word.len();
                // Skip the rest of the word.
                for _ in 1..word.chars().count() {
                    chars.next();
                }
            }
            _ => {}
        }
    }
    (quote.is_none() && depth == 0).then(|| {
        parts.push(&text[start..]);
        parts
    })
}

/// The inside of `( … )` when the parentheses enclose the whole text.
fn parenthesised(e: &str) -> Option<&str> {
    let inner = e.strip_prefix('(')?.strip_suffix(')')?;
    // `(a) and (b)` starts and ends with parentheses that don't enclose it all: its inside
    // doesn't balance. (A NUL never appears in a template, so nothing is cut.)
    split_top(inner, "\0").map(|_| inner)
}

fn simple(e: &str, named: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    match e {
        "True" => return Some("TRUE".into()),
        "False" | "None" => return Some("FALSE".into()),
        _ => {}
    }
    if let Some(f) = named(e) {
        return Some(f);
    }
    if let Some(p) = path(e) {
        return Some(truthy(p));
    }
    if let Some(f) = membership(e) {
        return Some(f);
    }
    // Longest operators first, so `>=` isn't read as `>`.
    for (py, formula) in [("==", "="), ("!=", "<>"), (">=", ">="), ("<=", "<="), (">", ">"), ("<", "<")] {
        if let Some((left, right)) = e.split_once(py) {
            if let (Some(p), Some(l)) = (path(left), literal(right)) {
                return Some(format!("[{p}] {formula} {l}"));
            }
            break;
        }
    }
    // Two fields, or a calculation, compared: `line.product_uom_id != line.product_id.uom_id`.
    arithmetic(e).filter(|f| COMPARISONS.iter().any(|c| f.contains(&format!(" {c} "))))
}

/// The comparison operators a formula written by `arithmetic` may hold.
const COMPARISONS: [&str; 6] = ["=", "<>", "<", ">", "<=", ">="];

/// A Python calculation over fields and numbers as a formula: `line.price_unit * 1.2` →
/// `[line.price_unit] * 1.2`, `o.amount_total - o.amount_residual`, `a ** 2` → `a ^ 2`,
/// `round(…, 2)`, `abs(…)`, `min(…)`, `max(…)`, and one comparison between two sides. `None` when
/// anything else appears (a method, `%`, `//`, text joined with `+`, a chained comparison) or
/// there is no operator nor function (a plain field is read elsewhere).
pub(crate) fn arithmetic(expr: &str) -> Option<String> {
    let mut out: Vec<String> = Vec::new();
    let (mut operators, mut comparisons, mut texts, mut plus) = (0, 0, false, false);
    let chars: Vec<char> = expr.trim().chars().collect();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        let next = chars.get(i + 1).copied();
        let token: String = match c {
            _ if c.is_whitespace() => {
                i += 1;
                continue;
            }
            '0'..='9' | '.' if c != '.' || next.is_some_and(|n| n.is_ascii_digit()) => {
                let start = i;
                while chars.get(i).is_some_and(|d| d.is_ascii_digit() || *d == '.' || *d == 'e' || *d == 'E') {
                    i += 1;
                }
                let n: String = chars.get(start..i)?.iter().collect();
                n.parse::<f64>().ok().filter(|x| x.is_finite())?;
                out.push(n);
                continue;
            }
            '\'' | '"' => {
                let end = chars.iter().skip(i + 1).position(|d| *d == c)? + i + 1;
                let text: String = chars.get(i + 1..end)?.iter().collect();
                if text.contains(['"', '\'', '\\']) {
                    return None;
                }
                texts = true;
                i = end + 1;
                out.push(format!("\"{text}\""));
                continue;
            }
            _ if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while chars.get(i).is_some_and(|d| d.is_ascii_alphanumeric() || *d == '_' || *d == '.') {
                    i += 1;
                }
                let name: String = chars.get(start..i)?.iter().collect();
                let call = chars.iter().skip(i).find(|d| !d.is_whitespace()) == Some(&'(');
                let token = if call {
                    operators += 1;
                    match name.as_str() {
                        "round" => "ROUND".to_owned(),
                        "abs" => "ABS".to_owned(),
                        "min" => "MIN".to_owned(),
                        "max" => "MAX".to_owned(),
                        _ => return None,
                    }
                } else {
                    format!("[{}]", path(&name)?)
                };
                out.push(token);
                continue;
            }
            '*' if next == Some('*') => {
                i += 1;
                "^".to_owned()
            }
            '=' | '!' | '<' | '>' => {
                let op = match (c, next) {
                    ('=', Some('=')) => "=",
                    ('!', Some('=')) => "<>",
                    ('<', Some('=')) => "<=",
                    ('>', Some('=')) => ">=",
                    ('<', _) => "<",
                    ('>', _) => ">",
                    _ => return None,
                };
                if next == Some('=') {
                    i += 1;
                }
                comparisons += 1;
                op.to_owned()
            }
            '/' if next == Some('/') => return None,
            '+' | '-' | '*' | '/' | '(' | ')' | ',' => {
                plus |= c == '+';
                c.to_string()
            }
            _ => return None,
        };
        if !matches!(token.as_str(), "(" | ")" | ",") {
            operators += 1;
        }
        out.push(token);
        i += 1;
    }
    if operators == 0 || comparisons > 1 || (texts && plus) {
        return None;
    }
    let formula = out.join(" ").replace("( ", "(").replace(" )", ")").replace(" ,", ",");
    let formula = ["ROUND", "ABS", "MIN", "MAX"].iter().fold(formula, |f, name| f.replace(&format!("{name} ("), &format!("{name}(")));
    // Only what the formula language reads back.
    reportcraft_expr::parse(&formula).ok().map(|_| formula)
}

/// `path in (a, b)` as `OR([path] = a, [path] = b)`, `path not in [a, b]` as
/// `AND([path] <> a, [path] <> b)`; the list holds numbers or quoted texts.
fn membership(e: &str) -> Option<String> {
    let (left, right, negated) = match split_top(e, " not in ")?.as_slice() {
        [l, r] => (*l, *r, true),
        _ => match split_top(e, " in ")?.as_slice() {
            [l, r] => (*l, *r, false),
            _ => return None,
        },
    };
    // A key of a dictionary (`'cash_rounding' in tax_totals`): there when it has a value.
    let key =
        left.trim().strip_prefix('\'').and_then(|k| k.strip_suffix('\'')).or_else(|| left.trim().strip_prefix('"').and_then(|k| k.strip_suffix('"')));
    if let (Some(k), Some(dict)) = (key, path(right)) {
        let field = path(&format!("{dict}.{k}"))?.to_owned();
        return Some(if negated { falsy(&field) } else { truthy(&field) });
    }
    let p = path(left)?;
    let list = right.trim();
    let inner = list.strip_prefix('(').and_then(|x| x.strip_suffix(')')).or_else(|| list.strip_prefix('[').and_then(|x| x.strip_suffix(']')))?;
    let items: Vec<&str> = split_top(inner, ",")?.into_iter().map(str::trim).filter(|i| !i.is_empty()).collect();
    let (op, function, empty) = if negated { ("<>", "AND", "TRUE") } else { ("=", "OR", "FALSE") };
    let tests: Option<Vec<String>> = items.iter().map(|i| literal(i).map(|l| format!("[{p}] {op} {l}"))).collect();
    let tests = tests?;
    Some(match tests.as_slice() {
        [] => empty.to_owned(),
        [one] => one.clone(),
        _ => format!("{function}({})", tests.join(", ")),
    })
}

/// A Python number or quoted text as a formula literal.
fn literal(text: &str) -> Option<String> {
    let t = text.trim();
    if let Ok(n) = t.parse::<f64>() {
        return n.is_finite().then(|| t.to_owned());
    }
    let quoted = t.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')).or_else(|| t.strip_prefix('"').and_then(|x| x.strip_suffix('"')))?;
    (!quoted.contains(['"', '\'', '\\'])).then(|| format!("\"{quoted}\""))
}

/// A field passed to one of Odoo's formatting helpers (`formatLang(env, o.amount)`,
/// `format_date(env, o.date)`), and whether it is a number (shown with two decimals).
pub(crate) fn formatted(expr: &str) -> Option<(&str, bool)> {
    let (name, args) = expr.trim().split_once('(')?;
    let args = args.strip_suffix(')')?;
    let numeric = match name.trim() {
        "formatLang" | "format_amount" | "format_number" | "format_float" => true,
        "format_date" | "format_datetime" => false,
        _ => return None,
    };
    if args.contains(['(', ')']) {
        return None;
    }
    let field = args.split(',').map(str::trim).filter(|a| *a != "env" && !a.contains('=')).find_map(path)?;
    Some((field, numeric))
}

/// Records' values joined into one text: `', '.join(line.tax_ids.mapped('name'))`,
/// `', '.join(t.tax_label for t in line.tax_ids if t.tax_label)`, `','.join(line.taxes)`. Gives
/// the column that holds them (`line.tax_ids.name`, `line.tax_ids.tax_label`, `line.taxes`) and the
/// separator. A filter (`if …`) is dropped: blanks are what it usually leaves out, and joining in
/// SQL leaves them out too.
pub(crate) fn joined(expr: &str) -> Option<(String, String)> {
    let e = expr.trim();
    let quote = e.chars().next().filter(|c| *c == '\'' || *c == '"')?;
    let close = e[1..].find(quote)? + 1;
    let separator = &e[1..close];
    let inner = e[close + 1..].trim_start().strip_prefix(".join(")?.strip_suffix(')')?.trim();
    if separator.contains('\\') {
        return None;
    }
    // `records.mapped('field')`
    if let Some((records, field)) = inner.strip_suffix("')").or_else(|| inner.strip_suffix("\")")).and_then(|i| i.split_once(".mapped(")) {
        let field = field.trim_matches(|c| c == '\'' || c == '"');
        let records = path(records)?;
        path(field)?;
        return Some((format!("{records}.{field}"), separator.to_owned()));
    }
    // `x.field for x in records [if …]`
    if let [value, rest] = split_top(inner, " for ")?.as_slice() {
        let rest = split_top(rest, " if ")?.first().copied()?;
        let parts = split_top(rest, " in ")?;
        let [var, records] = parts.as_slice() else { return None };
        let (var, records) = (var.trim(), path(records)?);
        let field = path(value)?.strip_prefix(var)?.strip_prefix('.')?;
        return Some((format!("{records}.{field}"), separator.to_owned()));
    }
    path(inner).map(|p| (p.to_owned(), separator.to_owned()))
}

/// The field a picture shows: `image_data_uri(o.image_128)`, Odoo's helper for a binary field.
pub(crate) fn image_field(src: &str) -> Option<&str> {
    let inner = src.trim().strip_prefix("image_data_uri(")?.strip_suffix(')')?;
    path(inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_blank_fallbacks() {
        assert_eq!(path("o.partner_id.name"), Some("o.partner_id.name"));
        assert_eq!(path(" line.name or '' "), Some("line.name"));
        assert_eq!(path("o.amount * 2"), None);
        assert_eq!(path("formatLang(env, o.amount)"), None);
        assert_eq!(path("o..name"), None);
        assert_eq!(path("1st"), None);
    }

    #[test]
    fn conditions_become_formulas() {
        assert_eq!(condition("o.note").as_deref(), Some("NOT(ISBLANK([o.note]))"));
        assert_eq!(condition("not o.note").as_deref(), Some("ISBLANK([o.note])"));
        assert_eq!(condition("o.state == 'posted'").as_deref(), Some("[o.state] = \"posted\""));
        assert_eq!(condition("line.discount >= 5").as_deref(), Some("[line.discount] >= 5"));
        assert_eq!(condition("o.state != \"draft\"").as_deref(), Some("[o.state] <> \"draft\""));
        assert_eq!(condition("o.a and not o.b").as_deref(), Some("AND(NOT(ISBLANK([o.a])), ISBLANK([o.b]))"));
        assert_eq!(condition("o.a or o.b and o.c > 1").as_deref(), Some("OR(NOT(ISBLANK([o.a])), AND(NOT(ISBLANK([o.b])), [o.c] > 1))"));
        assert_eq!(condition("o.a and o.f()"), None);
        assert_eq!(condition("o.s == 'x' and o.t").as_deref(), Some("AND([o.s] = \"x\", NOT(ISBLANK([o.t])))"));
        assert_eq!(condition("o.s == 'x and y'").as_deref(), Some("[o.s] = \"x and y\""));
        assert_eq!(condition("o.s in ['a', 'b']").as_deref(), Some("OR([o.s] = \"a\", [o.s] = \"b\")"));
        assert_eq!(condition("o.x == 'a\"b'"), None);
        assert_eq!(condition("o.x == y.z").as_deref(), Some("[o.x] = [y.z]"), "the builder checks `y` is a loop's variable");
    }

    #[test]
    fn parentheses_membership_and_named_conditions() {
        assert_eq!(
            condition("line.display_type in ('line_section', 'line_subsection')").as_deref(),
            Some("OR([line.display_type] = \"line_section\", [line.display_type] = \"line_subsection\")")
        );
        assert_eq!(condition("o.n not in (1, 2,)").as_deref(), Some("AND([o.n] <> 1, [o.n] <> 2)"));
        assert_eq!(condition("o.n in ('a',)").as_deref(), Some("[o.n] = \"a\""));
        assert_eq!(condition("o.n in ()").as_deref(), Some("FALSE"));
        assert_eq!(condition("o.n in (o.m, 1)"), None);
        assert_eq!(condition("o.a and (o.b or o.c)").as_deref(), Some("AND(NOT(ISBLANK([o.a])), OR(NOT(ISBLANK([o.b])), NOT(ISBLANK([o.c]))))"));
        assert_eq!(condition("(o.a) and (o.b)").as_deref(), Some("AND(NOT(ISBLANK([o.a])), NOT(ISBLANK([o.b])))"));
        assert_eq!(condition("not o.is_paid and o.show_total").as_deref(), Some("AND(NOT([o.is_paid]), [o.show_total])"));
        assert_eq!(condition("not (o.a or o.b)").as_deref(), Some("NOT(OR(NOT(ISBLANK([o.a])), NOT(ISBLANK([o.b]))))"));
        assert_eq!(condition("o.a and (o.b"), None);
        assert_eq!(condition(&"(".repeat(100)), None);
        let named = |n: &str| (n == "is_section").then(|| "[l.t] = \"s\"".to_owned());
        assert_eq!(condition_with("is_section", &named).as_deref(), Some("[l.t] = \"s\""));
        assert_eq!(condition_with("not is_section and l.x", &named).as_deref(), Some("AND(NOT([l.t] = \"s\"), NOT(ISBLANK([l.x])))"));
    }

    #[test]
    fn dictionary_keys_read_as_fields() {
        assert_eq!(keys("line['label']"), "line.label");
        assert_eq!(keys("t[\"amount\"] > 0 and x['a b']"), "t.amount > 0 and x['a b']");
        assert_eq!(keys("o.ids[0]"), "o.ids[0]");
        assert_eq!(keys("x['"), "x['");
        assert_eq!(keys("o.s in ['a']"), "o.s in ['a']");
        assert_eq!(
            keys("o.tax_totals and o.tax_totals.get('display_in_company_currency')"),
            "o.tax_totals and o.tax_totals.display_in_company_currency"
        );
        assert_eq!(keys("t.get('a', 0) + t.get(\"b\")"), "t.get('a', 0) + t.b");
        assert_eq!(keys("t.get("), "t.get(");
        assert_eq!(condition("'cash_rounding' in tax_totals").as_deref(), Some("NOT(ISBLANK([tax_totals.cash_rounding]))"));
        assert_eq!(condition("'a b' not in t").as_deref(), None);
        assert_eq!(condition("o.display_in_company_currency").as_deref(), Some("[o.display_in_company_currency]"));
        assert_eq!(condition("o.partner_id.display_name").as_deref(), Some("NOT(ISBLANK([o.partner_id.display_name]))"));
        assert_eq!(condition("not line.display_type").as_deref(), Some("ISBLANK([line.display_type])"));
    }

    #[test]
    fn formatting_helpers_show_their_field() {
        assert_eq!(formatted("formatLang(env, o.amount_total, currency_obj=o.currency_id)"), Some(("o.amount_total", true)));
        assert_eq!(formatted("format_number(move.quantity)"), Some(("move.quantity", true)));
        assert_eq!(formatted("format_date(env, o.date)"), Some(("o.date", false)));
        assert_eq!(formatted("format_date(env, o.f())"), None);
        assert_eq!(formatted("len(o.line_ids)"), None);
        assert_eq!(condition("False").as_deref(), Some("FALSE"));
    }

    #[test]
    fn calculations_on_fields_become_formulas() {
        assert_eq!(arithmetic("line.price_unit * 1.2").as_deref(), Some("[line.price_unit] * 1.2"));
        assert_eq!(arithmetic("o.amount_total - o.amount_residual").as_deref(), Some("[o.amount_total] - [o.amount_residual]"));
        assert_eq!(
            arithmetic("round(line.qty * (1 - line.discount / 100), 2)").as_deref(),
            Some("ROUND([line.qty] * (1 - [line.discount] / 100), 2)")
        );
        assert_eq!(arithmetic("abs(o.amount) ** 2").as_deref(), Some("ABS([o.amount]) ^ 2"));
        assert_eq!(arithmetic("-o.amount").as_deref(), Some("- [o.amount]"));
        assert_eq!(arithmetic("line.a != line.b").as_deref(), Some("[line.a] <> [line.b]"));
        assert_eq!(arithmetic("o.name"), None, "a plain field");
        assert_eq!(arithmetic("o.f(1) + 1"), None);
        assert_eq!(arithmetic("o.n % 2"), None);
        assert_eq!(arithmetic("o.n // 2"), None);
        assert_eq!(arithmetic("'a' + o.name"), None);
        assert_eq!(arithmetic("1 < o.n < 3"), None);
        assert_eq!(arithmetic("o.a * (o.b"), None);
        assert_eq!(arithmetic("o.a + 1e999"), None);
        assert_eq!(condition("line.product_uom_id != line.product_id.uom_id").as_deref(), Some("[line.product_uom_id] <> [line.product_id.uom_id]"));
        assert_eq!(condition("o.amount_total > o.amount_residual * 2").as_deref(), Some("[o.amount_total] > [o.amount_residual] * 2"));
        assert_eq!(condition("o.state == 'posted'").as_deref(), Some("[o.state] = \"posted\""));
    }

    #[test]
    fn joined_records_read_as_one_text_column() {
        let j = |e: &str| joined(e).map(|(c, s)| format!("{c}|{s}"));
        assert_eq!(j("', '.join(line.tax_ids.mapped('name'))").as_deref(), Some("line.tax_ids.name|, "));
        assert_eq!(j("', '.join(tax.tax_label for tax in line.tax_ids if tax.tax_label)").as_deref(), Some("line.tax_ids.tax_label|, "));
        assert_eq!(j("\" / \".join(s.lot_id.name for s in move.move_line_ids)").as_deref(), Some("move.move_line_ids.lot_id.name| / "));
        assert_eq!(j("','.join(grouped_line.taxes)").as_deref(), Some("grouped_line.taxes|,"));
        assert_eq!(j("', '.join(str(t.amount) for t in line.tax_ids)"), None);
        assert_eq!(j("', '.join(x.name for y in line.tax_ids)"), None);
        assert_eq!(j("', '.join(line.tax_ids.mapped(lambda t: t.name))"), None);
        assert_eq!(j("line.name"), None);
        assert_eq!(j("'"), None);
    }

    #[test]
    fn pictures_from_binary_fields() {
        assert_eq!(image_field("image_data_uri(o.company_id.logo)"), Some("o.company_id.logo"));
        assert_eq!(image_field("'/web/image/' + str(o.id)"), None);
    }
}
