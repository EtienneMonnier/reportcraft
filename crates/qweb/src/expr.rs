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
/// `show_payment_term_details`): Python's truth of a flag is its value, not whether it is blank.
pub(crate) fn flag(path: &str) -> bool {
    let leaf = path.rsplit('.').next().unwrap_or(path).to_ascii_lowercase();
    ["is_", "has_", "show_", "hide_", "collapse_", "display_on_"].iter().any(|p| leaf.starts_with(p))
}

/// `path` true, as Python reads it: a flag's value, any other field's being there.
fn truthy(p: &str) -> String {
    if flag(p) { format!("[{p}]") } else { format!("NOT(ISBLANK([{p}]))") }
}

/// `path` false, as Python reads it.
fn falsy(p: &str) -> String {
    if flag(p) { format!("NOT([{p}])") } else { format!("ISBLANK([{p}])") }
}

/// Dictionary keys written as fields: `line['label']` reads as `line.label` (Odoo's grouped
/// lines and tax totals are dictionaries).
pub(crate) fn keys(e: &str) -> String {
    let mut out = String::with_capacity(e.len());
    let mut rest = e;
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
            let p = path(left)?;
            return Some(format!("[{p}] {formula} {}", literal(right)?));
        }
    }
    None
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
        assert_eq!(condition("o.x == y.z"), None);
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
    fn pictures_from_binary_fields() {
        assert_eq!(image_field("image_data_uri(o.company_id.logo)"), Some("o.company_id.logo"));
        assert_eq!(image_field("'/web/image/' + str(o.id)"), None);
    }
}
