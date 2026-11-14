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

/// Dictionary keys written as fields: `line['label']` reads as `line.label` (Odoo's grouped
/// lines and tax totals are dictionaries).
pub(crate) fn keys(e: &str) -> String {
    let mut out = String::with_capacity(e.len());
    let mut rest = e;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let quote = after.chars().next().filter(|c| *c == '\'' || *c == '"');
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
/// (`o.note`, `not o.note`) and a comparison with a number or a quoted text
/// (`o.state == 'posted'`, `line.discount > 0`). `None` for anything else.
pub(crate) fn condition(expr: &str) -> Option<String> {
    let e = expr.trim();
    // `a or b and c` reads as `a or (b and c)`, as in Python; parentheses aren't read.
    if !e.contains(['(', ')', '[']) {
        for (word, function) in [(" or ", "OR"), (" and ", "AND")] {
            if e.contains(word) {
                // A quoted text holding the word would be cut in two: refused.
                let quoted = |p: &str| p.matches('\'').count() % 2 == 1 || p.matches('"').count() % 2 == 1;
                if e.split(word).any(quoted) {
                    return None;
                }
                let parts: Option<Vec<String>> = e.split(word).map(condition).collect();
                return parts.map(|p| format!("{function}({})", p.join(", ")));
            }
        }
    }
    simple(e)
}

fn simple(e: &str) -> Option<String> {
    match e {
        "True" => return Some("TRUE".into()),
        "False" | "None" => return Some("FALSE".into()),
        _ => {}
    }
    if let Some(p) = e.strip_prefix("not ").and_then(path) {
        return Some(format!("ISBLANK([{p}])"));
    }
    if let Some(p) = path(e) {
        return Some(format!("NOT(ISBLANK([{p}]))"));
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
        assert_eq!(condition("o.s == 'x and y'"), None);
        assert_eq!(condition("o.s in ['a', 'b']"), None);
        assert_eq!(condition("o.x == 'a\"b'"), None);
        assert_eq!(condition("o.x == y.z"), None);
    }

    #[test]
    fn dictionary_keys_read_as_fields() {
        assert_eq!(keys("line['label']"), "line.label");
        assert_eq!(keys("t[\"amount\"] > 0 and x['a b']"), "t.amount > 0 and x['a b']");
        assert_eq!(keys("o.ids[0]"), "o.ids[0]");
        assert_eq!(keys("x['"), "x['");
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
