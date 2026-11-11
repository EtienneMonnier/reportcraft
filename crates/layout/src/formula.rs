//! Formulas in a render: named formulas become columns of every row, formula elements are
//! evaluated per band against the current row and the rows the band summarises.

use std::cell::RefCell;
use std::collections::HashMap;

use chrono::NaiveDateTime;
use reportcraft_core::{Cancel, Column, Error, FieldType, Result, RowSet, Value};
use reportcraft_expr::{Env, Expr, ExprError, Val, evaluate, parse};
use reportcraft_model::{ParamType, ParamValue, Report};

use crate::{PRINTED_AT, Params};

/// Rows computed between two looks at the cancel flag.
const CANCEL_EVERY: usize = 1_024;

/// A data value as a formula value.
pub(crate) fn from_value(v: &Value) -> Val {
    match v {
        Value::Null => Val::Blank,
        Value::Bool(b) => Val::Bool(*b),
        Value::Int(i) => Val::Number(*i as f64),
        Value::Number(n) => Val::number(*n),
        Value::Text(t) => Val::Text(t.clone()),
    }
}

/// A formula value as a data value: numbers kept to 15 significant digits, dates as ISO text
/// (what a date format reads), errors as their text (`#DIV/0!`).
pub(crate) fn to_value(v: Val) -> Value {
    match v {
        Val::Blank => Value::Null,
        Val::Bool(b) => Value::Bool(b),
        Val::Number(n) => Value::Number(reportcraft_expr::significant(n)),
        Val::Text(t) => Value::Text(t),
        other @ (Val::Date(_) | Val::DateTime(_) | Val::Error(_)) => Value::Text(other.to_string()),
    }
}

/// The column a formula's field means: `[order.date]` is `order.date`, else `date`.
pub(crate) fn resolve(rows: &RowSet, name: &str) -> Option<usize> {
    reportcraft_expr::field_candidates(name).find_map(|n| rows.column_index(n))
}

/// A parameter's value, typed by its declaration; `name.from` and `name.to` are the ends of a
/// date range.
pub(crate) fn param(report: &Report, params: &Params, name: &str) -> Val {
    let (base, end) = match name.rsplit_once('.') {
        Some((base, end @ ("from" | "to"))) => (base, Some(end)),
        _ => (name, None),
    };
    let declared = report.parameters.iter().find(|p| p.name == base);
    let raw = params.get(base).cloned().or_else(|| declared.and_then(|p| p.default.clone()));
    let Some(raw) = raw else { return Val::Blank };
    let Some(p) = declared.filter(|p| !p.multiple) else { return Val::Text(raw) };
    match (p.parse(&raw), end) {
        (Ok(ParamValue::DateRange(from, _)), Some("from")) => Val::Date(from),
        (Ok(ParamValue::DateRange(_, to)), Some("to")) => Val::Date(to),
        (Ok(_), Some(_)) => Val::Blank,
        (Ok(ParamValue::Integer(i)), None) => Val::Number(i as f64),
        (Ok(ParamValue::Number(n)), None) => Val::number(n),
        (Ok(ParamValue::Boolean(b)), None) => Val::Bool(b),
        (Ok(ParamValue::Date(d)), None) if p.kind == ParamType::Date => Val::Date(d),
        _ => Val::Text(raw),
    }
}

/// How the report's formulas write numbers and dates as text.
pub(crate) fn locale_of(report: &Report) -> reportcraft_expr::Locale {
    let style = reportcraft_model::locale_or_default(&report.locale);
    reportcraft_expr::Locale { decimal: style.decimal, group: style.group, names: style.names }
}

/// When the report is rendered, from the render values.
pub(crate) fn now(params: &Params) -> Option<NaiveDateTime> {
    params.get(PRINTED_AT).and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|dt| dt.naive_local())
}

/// What a named formula reads: one row being computed.
struct RowEnv<'a> {
    report: &'a Report,
    rows: &'a RowSet,
    row: &'a [Value],
    params: &'a Params,
    now: Option<NaiveDateTime>,
    locale: reportcraft_expr::Locale,
}

impl Env for RowEnv<'_> {
    fn field(&self, name: &str) -> Val {
        resolve(self.rows, name).map_or(Val::Error(ExprError::Name), |c| from_value(self.rows.value(self.row, c)))
    }

    fn range(&self, _: &str) -> std::result::Result<Vec<Val>, ExprError> {
        Err(ExprError::Value)
    }

    fn param(&self, name: &str) -> Val {
        param(self.report, self.params, name)
    }

    fn now(&self) -> Option<NaiveDateTime> {
        self.now
    }

    fn locale(&self) -> reportcraft_expr::Locale {
        self.locale
    }
}

/// The rows with one more column per named formula, or `None` when the report has none.
pub(crate) fn with_formulas(report: &Report, rows: &RowSet, params: &Params, cancel: &Cancel) -> Result<Option<RowSet>> {
    if report.formulas.is_empty() {
        return Ok(None);
    }
    let order = report.formula_order()?;
    let exprs = report
        .formulas
        .iter()
        .map(|f| parse(&f.expression).map_err(|e| Error::Invalid(format!("formula `{}`: {e}", f.name))))
        .collect::<Result<Vec<Expr>>>()?;
    let base = rows.columns.len();
    let mut out = RowSet {
        columns: rows
            .columns
            .iter()
            .cloned()
            .chain(report.formulas.iter().map(|f| Column { name: f.name.clone(), field_type: FieldType::Text }))
            .collect(),
        rows: Vec::with_capacity(rows.rows.len()),
    };
    let now = now(params);
    let locale = locale_of(report);
    for (n, row) in rows.rows.iter().enumerate() {
        if n % CANCEL_EVERY == 0 {
            cancel.check("the render")?;
        }
        let mut values = Vec::with_capacity(out.columns.len());
        values.extend(row.iter().take(base).cloned());
        values.resize(out.columns.len(), Value::Null);
        for &i in &order {
            let v = match exprs.get(i) {
                Some(e) => evaluate(e, &RowEnv { report, rows: &out, row: &values, params, now, locale }),
                None => Val::Blank,
            };
            if let Some(slot) = values.get_mut(base + i) {
                *slot = to_value(v);
            }
        }
        out.rows.push(values);
    }
    // A column's type is what its values turned out to be.
    for (i, col) in out.columns.iter_mut().enumerate().skip(base) {
        let mut kinds = out.rows.iter().filter_map(|r| r.get(i)).filter(|v| !v.is_null());
        col.field_type = match kinds.next() {
            Some(first) => {
                let t = |v: &Value| match v {
                    Value::Int(_) | Value::Number(_) => FieldType::Number,
                    Value::Bool(_) => FieldType::Bool,
                    _ => FieldType::Text,
                };
                let ty = t(first);
                if kinds.all(|v| t(v) == ty) { ty } else { FieldType::Text }
            }
            None => FieldType::Text,
        };
    }
    Ok(Some(out))
}

/// The last aggregate results, for the rows being summarised (one scope at a time: the rows of
/// a detail band's group share it).
#[derive(Default)]
pub(crate) struct Memo {
    scope: RefCell<(usize, usize, HashMap<usize, Val>)>,
}

/// What a formula element reads: the band's current row and the rows it summarises.
pub(crate) struct BandEnv<'a> {
    pub report: &'a Report,
    pub rows: &'a RowSet,
    pub current: Option<usize>,
    pub summary: &'a [usize],
    pub params: &'a Params,
    pub now: Option<NaiveDateTime>,
    pub memo: &'a Memo,
    pub locale: reportcraft_expr::Locale,
}

impl Env for BandEnv<'_> {
    fn field(&self, name: &str) -> Val {
        match (resolve(self.rows, name), self.current) {
            (None, _) => Val::Error(ExprError::Name),
            (Some(_), None) => Val::Blank,
            (Some(c), Some(r)) => from_value(self.rows.value(self.rows.rows.get(r).map_or(&[], Vec::as_slice), c)),
        }
    }

    fn range(&self, name: &str) -> std::result::Result<Vec<Val>, ExprError> {
        let c = resolve(self.rows, name).ok_or(ExprError::Name)?;
        Ok(self.summary.iter().map(|&r| from_value(self.rows.value(self.rows.rows.get(r).map_or(&[], Vec::as_slice), c))).collect())
    }

    fn param(&self, name: &str) -> Val {
        param(self.report, self.params, name)
    }

    fn now(&self) -> Option<NaiveDateTime> {
        self.now
    }

    fn locale(&self) -> reportcraft_expr::Locale {
        self.locale
    }

    fn memo(&self, key: usize, compute: &mut dyn FnMut() -> Val) -> Val {
        let scope = (self.summary.as_ptr() as usize, self.summary.len());
        if let Ok(mut m) = self.memo.scope.try_borrow_mut() {
            if (m.0, m.1) != scope {
                *m = (scope.0, scope.1, HashMap::new());
            }
            if let Some(v) = m.2.get(&key) {
                return v.clone();
            }
        }
        let v = compute();
        if let Ok(mut m) = self.memo.scope.try_borrow_mut()
            && (m.0, m.1) == scope
        {
            m.2.insert(key, v.clone());
        }
        v
    }
}
