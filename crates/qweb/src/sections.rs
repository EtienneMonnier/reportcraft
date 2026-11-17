//! Odoo's lines of type section and subsection (`display_type == 'line_section'`) are titles
//! over the lines that follow them, with the total of those lines. A banded report shows that as
//! groups: the lines are grouped by section (and by subsection inside it), each group's header
//! is the section's row, and a total in it sums the group's lines. The data says which section a
//! line falls under with a number per document (0 before the first section, then 1, 2…, the
//! section's own line included), and which subsection within it.

use reportcraft_core::FieldType;
use reportcraft_expr::{Expr, Func, Op};
use reportcraft_model::{Band, Group, Report};

use crate::flow::{Field, Level, add_field};

/// The kinds of title lines, with the column numbering them.
const KINDS: [(&str, &str); 2] = [("line_section", "section_number"), ("line_subsection", "subsection_number")];

/// Groups the detail by section, and subsection, when the template shows rows of its own for
/// them. Returns the numbering columns added.
pub(crate) fn apply(report: &mut Report, fields: &mut Vec<Field>) -> Vec<String> {
    let Some(kind_field) = fields.iter().find(|f| f.level == Level::Line && f.name.ends_with(".display_type")).map(|f| f.name.clone()) else {
        return Vec::new();
    };
    let var = kind_field.trim_end_matches(".display_type").to_owned();
    let mut added = Vec::new();
    let mut hide = Vec::new();
    for (kind, number) in KINDS {
        let shown = |cond: Option<&str>, value: Option<&str>| truth(cond, &kind_field, value) != Some(false);
        // Content of its own: shown on this kind's row, hidden on a product's (or on a line with
        // no type, as older versions write products).
        let own = report.detail.elements.iter().any(|e| {
            let c = e.visible_if.as_deref();
            shown(c, Some(kind)) && (!shown(c, Some("product")) || !shown(c, None))
        });
        if !own {
            continue;
        }
        let mut elements: Vec<_> = report.detail.elements.iter().filter(|e| shown(e.visible_if.as_deref(), Some(kind))).cloned().collect();
        // Odoo prints section titles bold.
        for e in &mut elements {
            e.style.bold = true;
        }
        let test = format!("[{kind_field}] = \"{kind}\"");
        let header = Band { height_mm: report.detail.height_mm, elements, visible_if: Some(test), ..Band::default() };
        let key = format!("{var}.{number}");
        add_field(fields, &key, Level::Line);
        if let Some(f) = fields.iter_mut().find(|f| f.name == key) {
            f.field_type = FieldType::Integer;
        }
        report.groups.push(Group {
            field: key.clone(),
            keep_together: false,
            repeat_header: false,
            page_break_before: false,
            header,
            footer: Band::default(),
        });
        hide.push(format!("[{kind_field}] <> \"{kind}\""));
        added.push(key);
    }
    if !hide.is_empty() {
        // The title lines are the groups' headers: their own rows take no room.
        hide.extend(report.detail.visible_if.take());
        report.detail.visible_if = Some(match hide.as_slice() {
            [one] => one.clone(),
            _ => format!("AND({})", hide.join(", ")),
        });
    }
    added
}

/// What a value is known to be when only the line's type is known.
#[derive(Debug, Clone, PartialEq)]
enum Known {
    Bool(bool),
    Number(f64),
    Text(String),
    Blank,
    Unknown,
}

/// Whether a condition holds on a line whose `field` is `value` (`None`: blank), whatever its
/// other fields: `Some(true)` or `Some(false)` when that alone decides, `None` otherwise. No
/// condition always holds; one that doesn't parse is undecided.
fn truth(condition: Option<&str>, field: &str, value: Option<&str>) -> Option<bool> {
    let Some(text) = condition else { return Some(true) };
    let expr = reportcraft_expr::parse(text).ok()?;
    match eval(&expr, field, value, 0) {
        Known::Bool(b) => Some(b),
        Known::Number(n) => Some(n != 0.0),
        Known::Blank => Some(false),
        Known::Text(_) | Known::Unknown => None,
    }
}

fn eval(e: &Expr, field: &str, value: Option<&str>, depth: usize) -> Known {
    if depth > reportcraft_expr::MAX_DEPTH {
        return Known::Unknown;
    }
    let go = |x: &Expr| eval(x, field, value, depth + 1);
    match e {
        Expr::Bool(b) => Known::Bool(*b),
        Expr::Number(n) => Known::Number(*n),
        Expr::Text(t) => Known::Text(t.clone()),
        Expr::Field(f) if f == field => value.map_or(Known::Blank, |v| Known::Text(v.to_owned())),
        Expr::Binary(op @ (Op::Eq | Op::Ne), a, b) => {
            let text = |k: Known| match k {
                Known::Text(t) => Some(t),
                Known::Blank => Some(String::new()),
                _ => None,
            };
            match (text(go(a)), text(go(b))) {
                (Some(x), Some(y)) => Known::Bool(x.eq_ignore_ascii_case(&y) == (*op == Op::Eq)),
                _ => Known::Unknown,
            }
        }
        Expr::Call { func: Func::Not, args, .. } => match args.first().map(go) {
            Some(Known::Bool(b)) => Known::Bool(!b),
            _ => Known::Unknown,
        },
        Expr::Call { func: Func::IsBlank, args, .. } => match args.first().map(go) {
            Some(Known::Blank) => Known::Bool(true),
            Some(Known::Text(t)) => Known::Bool(t.is_empty()),
            Some(Known::Bool(_) | Known::Number(_)) => Known::Bool(false),
            _ => Known::Unknown,
        },
        Expr::Call { func: func @ (Func::And | Func::Or), args, .. } => {
            // AND is false as soon as one part is, OR true as soon as one part is.
            let decisive = *func == Func::Or;
            let mut all_known = true;
            for arg in args {
                match go(arg) {
                    Known::Bool(b) if b == decisive => return Known::Bool(decisive),
                    Known::Bool(_) => {}
                    _ => all_known = false,
                }
            }
            if all_known { Known::Bool(!decisive) } else { Known::Unknown }
        }
        _ => Known::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions_decided_by_the_line_type_alone() {
        let t = |c: &str, v: Option<&str>| truth(Some(c), "l.display_type", v);
        let section = Some("line_section");
        assert_eq!(t("[l.display_type] = \"line_section\"", section), Some(true));
        assert_eq!(t("[l.display_type] = \"product\"", section), Some(false));
        assert_eq!(t("AND(ISBLANK([l.x]), [l.display_type] = \"product\")", section), Some(false));
        assert_eq!(t("AND(ISBLANK([l.x]), [l.display_type] = \"line_section\")", section), None);
        assert_eq!(t("OR([l.x] = 1, [l.display_type] = \"line_section\")", section), Some(true));
        assert_eq!(t("NOT(ISBLANK([l.display_type]))", None), Some(false));
        assert_eq!(t("ISBLANK([l.display_type])", Some("product")), Some(false));
        assert_eq!(t("AND(NOT([l.display_type] = \"product\"), NOT(OR([l.display_type] = \"line_section\")))", section), Some(false));
        assert_eq!(t("[l.amount] > 0", section), None);
        assert_eq!(t("not a formula (", section), None);
        assert_eq!(truth(None, "l.display_type", section), Some(true));
    }
}
