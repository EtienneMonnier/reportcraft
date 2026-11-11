//! Evaluates a parsed formula against what the caller provides: the current row, the rows being
//! summarised, the report's parameters and the render's clock.

use std::cmp::Ordering;

use chrono::{Datelike, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Timelike};

use crate::parse::{Expr, Op};
use crate::value::{ExprError, R, Val, compare, parse_datetime, parse_number, significant};
use crate::{Func, MAX_TEXT};

/// What a formula can read. The caller decides what "the current row" and "the rows being
/// summarised" are (a detail row and its group, a group footer and its rows…).
pub trait Env {
    /// The current row's value of `name`; `Val::Error(ExprError::Name)` when there is no such
    /// field, `Val::Blank` when there is no current row.
    fn field(&self, name: &str) -> Val;
    /// `name` over the rows being summarised. `Err(Name)` for an unknown field, `Err(Value)`
    /// where nothing is summarised (a row formula).
    fn range(&self, name: &str) -> Result<Vec<Val>, ExprError>;
    /// A parameter's value; blank when it has none.
    fn param(&self, name: &str) -> Val;
    /// When the report is being rendered; `None` gives `#N/A` for `TODAY()` and `NOW()`.
    fn now(&self) -> Option<NaiveDateTime>;
    /// Remembers the result of an aggregate that doesn't depend on the current row, keyed by the
    /// call's identity, for the rows being summarised. The default doesn't remember.
    fn memo(&self, key: usize, compute: &mut dyn FnMut() -> Val) -> Val {
        let _ = key;
        compute()
    }
}

/// The formula's value. Never panics; every failure is an error value.
pub fn evaluate(expr: &Expr, env: &dyn Env) -> Val {
    match expr {
        Expr::Number(n) => Val::number(*n),
        Expr::Text(t) => Val::Text(t.clone()),
        Expr::Bool(b) => Val::Bool(*b),
        Expr::Field(name) => env.field(name),
        // A range outside an aggregate has no single value.
        Expr::Range(_) => Val::Error(ExprError::Value),
        Expr::Neg(a) => wrap(evaluate(a, env).to_number().map(|n| Val::number(-n))),
        Expr::Percent(a) => wrap(evaluate(a, env).to_number().map(|n| Val::number(n / 100.0))),
        Expr::Binary(op, a, b) => binary(*op, evaluate(a, env), evaluate(b, env)),
        Expr::Call { func, args, row_independent } => {
            if *row_independent && is_aggregate(*func) {
                let key = std::ptr::from_ref(expr) as usize;
                env.memo(key, &mut || call(*func, args, env))
            } else {
                call(*func, args, env)
            }
        }
    }
}

fn wrap(r: R<Val>) -> Val {
    r.unwrap_or_else(Val::Error)
}

fn is_aggregate(f: Func) -> bool {
    matches!(f, Func::Sum | Func::Average | Func::Count | Func::CountA | Func::Min | Func::Max | Func::SumIf | Func::CountIf | Func::AverageIf)
}

fn text(s: String) -> R<Val> {
    if s.len() > MAX_TEXT { Err(ExprError::Value) } else { Ok(Val::Text(s)) }
}

fn binary(op: Op, a: Val, b: Val) -> Val {
    wrap(binary_r(op, &a, &b))
}

fn binary_r(op: Op, a: &Val, b: &Val) -> R<Val> {
    a.err()?;
    b.err()?;
    let cmp = |want: fn(Ordering) -> bool| compare(a, b).map(|o| Val::Bool(want(o)));
    match op {
        Op::Add | Op::Sub if a.is_date_like() || b.is_date_like() => date_arithmetic(op, a, b),
        Op::Add => Ok(Val::number(a.to_number()? + b.to_number()?)),
        Op::Sub => Ok(Val::number(a.to_number()? - b.to_number()?)),
        Op::Mul => Ok(Val::number(a.to_number()? * b.to_number()?)),
        Op::Div => {
            let (x, y) = (a.to_number()?, b.to_number()?);
            if y == 0.0 { Err(ExprError::Div0) } else { Ok(Val::number(x / y)) }
        }
        Op::Pow => {
            let (x, y) = (a.to_number()?, b.to_number()?);
            if x == 0.0 && y <= 0.0 { Err(if y == 0.0 { ExprError::Num } else { ExprError::Div0 }) } else { Ok(Val::number(x.powf(y))) }
        }
        Op::Concat => {
            let (x, y) = (a.to_text()?, b.to_text()?);
            if x.len().saturating_add(y.len()) > MAX_TEXT {
                return Err(ExprError::Value);
            }
            Ok(Val::Text(x + &y))
        }
        Op::Eq => cmp(|o| o == Ordering::Equal),
        Op::Ne => cmp(|o| o != Ordering::Equal),
        Op::Lt => cmp(|o| o == Ordering::Less),
        Op::Gt => cmp(|o| o == Ordering::Greater),
        Op::Le => cmp(|o| o != Ordering::Greater),
        Op::Ge => cmp(|o| o != Ordering::Less),
    }
}

/// A date plus or minus days, or the days between two dates.
fn date_arithmetic(op: Op, a: &Val, b: &Val) -> R<Val> {
    match (a.is_date_like(), b.is_date_like()) {
        (true, true) if op == Op::Sub => {
            let (x, y) = (a.as_datetime().ok_or(ExprError::Value)?, b.as_datetime().ok_or(ExprError::Value)?);
            let seconds = (x - y).num_milliseconds() as f64 / 1000.0;
            Ok(Val::number(significant(seconds / 86_400.0)))
        }
        (true, false) => shift(a, if op == Op::Sub { -b.to_number()? } else { b.to_number()? }),
        (false, true) if op == Op::Add => shift(b, a.to_number()?),
        _ => Err(ExprError::Value),
    }
}

/// A date moved by a number of days; whole days on a date give a date.
fn shift(date: &Val, days: f64) -> R<Val> {
    let start = date.as_datetime().ok_or(ExprError::Value)?;
    let has_time = matches!(date, Val::DateTime(_)) || start.time() != NaiveTime::MIN;
    if days.abs() > 3_000_000.0 {
        return Err(ExprError::Num);
    }
    let ms = (days * 86_400_000.0).round() as i64;
    let moved = start.checked_add_signed(TimeDelta::try_milliseconds(ms).ok_or(ExprError::Num)?).ok_or(ExprError::Num)?;
    Ok(if !has_time && days.fract() == 0.0 { Val::Date(moved.date()) } else { Val::DateTime(moved) })
}

fn call(func: Func, args: &[Expr], env: &dyn Env) -> Val {
    let arg = |i: usize| args.get(i).map_or(Val::Blank, |a| evaluate(a, env));
    wrap(match func {
        Func::If | Func::Ifs | Func::Switch | Func::And | Func::Or | Func::Not | Func::IfError => logical(func, args, env),
        Func::IsBlank => Ok(Val::Bool(matches!(arg(0), Val::Blank))),
        Func::IsError => Ok(Val::Bool(arg(0).is_error())),
        Func::IsNumber => Ok(Val::Bool(matches!(arg(0), Val::Number(_)))),
        Func::IsText => Ok(Val::Bool(matches!(arg(0), Val::Text(_)))),
        Func::Abs | Func::Round | Func::RoundUp | Func::RoundDown | Func::Int | Func::Mod | Func::Power | Func::Sqrt | Func::Sign => {
            maths(func, &arg(0), &arg(1))
        }
        Func::Sum | Func::Average | Func::Count | Func::CountA | Func::Min | Func::Max => aggregate(func, args, env),
        Func::SumIf | Func::CountIf | Func::AverageIf => conditional(func, args, env),
        Func::Concat => concat(args, env),
        Func::Len
        | Func::Left
        | Func::Right
        | Func::Mid
        | Func::Upper
        | Func::Lower
        | Func::Proper
        | Func::Trim
        | Func::Substitute
        | Func::Find
        | Func::Search
        | Func::Rept
        | Func::Exact
        | Func::Value => {
            let values: Vec<Val> = (0..args.len()).map(arg).collect();
            text_function(func, &values)
        }
        Func::Today => env.now().map(|n| Val::Date(n.date())).ok_or(ExprError::Na),
        Func::Now => env.now().map(Val::DateTime).ok_or(ExprError::Na),
        Func::Param => match args.first() {
            Some(Expr::Text(name)) => Ok(env.param(name)),
            _ => Err(ExprError::Value),
        },
        Func::Date
        | Func::DateValue
        | Func::Year
        | Func::Month
        | Func::Day
        | Func::Hour
        | Func::Minute
        | Func::Weekday
        | Func::IsoWeekNum
        | Func::EDate
        | Func::EoMonth
        | Func::Days
        | Func::DateDif => {
            let values: Vec<Val> = (0..args.len()).map(arg).collect();
            date_function(func, &values)
        }
    })
}

fn concat(args: &[Expr], env: &dyn Env) -> R<Val> {
    let mut out = String::new();
    for a in args {
        out.push_str(&evaluate(a, env).to_text()?);
        if out.len() > MAX_TEXT {
            return Err(ExprError::Value);
        }
    }
    Ok(Val::Text(out))
}

/// `IF`, `IFS`, `SWITCH`, `AND`, `OR`, `NOT`, `IFERROR`: only the branches needed are evaluated.
fn logical(func: Func, args: &[Expr], env: &dyn Env) -> R<Val> {
    let arg = |i: usize| args.get(i).map_or(Val::Blank, |a| evaluate(a, env));
    match func {
        Func::If => {
            if arg(0).to_bool()? {
                Ok(arg(1))
            } else {
                Ok(if args.len() > 2 { arg(2) } else { Val::Bool(false) })
            }
        }
        Func::Ifs => {
            for pair in args.chunks(2) {
                if let [cond, value] = pair
                    && evaluate(cond, env).to_bool()?
                {
                    return Ok(evaluate(value, env));
                }
            }
            Err(ExprError::Na)
        }
        Func::Switch => {
            let subject = arg(0);
            subject.err()?;
            let rest = args.get(1..).unwrap_or_default();
            for pair in rest.chunks(2) {
                match pair {
                    [candidate, result] => {
                        if compare(&subject, &evaluate(candidate, env)).ok() == Some(Ordering::Equal) {
                            return Ok(evaluate(result, env));
                        }
                    }
                    [default] => return Ok(evaluate(default, env)),
                    _ => {}
                }
            }
            Err(ExprError::Na)
        }
        Func::And | Func::Or => {
            let mut all = true;
            let mut any = false;
            for a in args {
                let b = evaluate(a, env).to_bool()?;
                all &= b;
                any |= b;
            }
            Ok(Val::Bool(if func == Func::And { all } else { any }))
        }
        Func::Not => Ok(Val::Bool(!arg(0).to_bool()?)),
        Func::IfError => {
            let v = arg(0);
            Ok(if v.is_error() { arg(1) } else { v })
        }
        _ => Err(ExprError::Value),
    }
}

/// Rounds `n` to `digits` (negative: tens, hundreds…), after the 15 significant digits Excel
/// keeps, so `ROUND(2.675, 2)` is 2.68 as it is written.
fn round_with(n: f64, digits: f64, how: fn(f64) -> f64) -> R<Val> {
    let digits = digits.trunc().clamp(-15.0, 15.0) as i32;
    let scale = 10f64.powi(digits.abs());
    let scaled = if digits >= 0 { significant(n * scale) } else { significant(n / scale) };
    let r = how(scaled);
    Ok(Val::number(if digits >= 0 { r / scale } else { r * scale }))
}

fn maths(func: Func, a: &Val, b: &Val) -> R<Val> {
    let x = a.to_number()?;
    match func {
        Func::Abs => Ok(Val::number(x.abs())),
        Func::Round => round_with(x, b.to_number()?, |v| v.round()),
        Func::RoundUp => round_with(x, b.to_number()?, |v| if v < 0.0 { v.floor() } else { v.ceil() }),
        Func::RoundDown => round_with(x, b.to_number()?, f64::trunc),
        Func::Int => Ok(Val::number(x.floor())),
        Func::Mod => {
            let y = b.to_number()?;
            if y == 0.0 { Err(ExprError::Div0) } else { Ok(Val::number(x - y * (x / y).floor())) }
        }
        Func::Power => binary_r(Op::Pow, &Val::Number(x), b),
        Func::Sqrt => {
            if x < 0.0 {
                Err(ExprError::Num)
            } else {
                Ok(Val::number(x.sqrt()))
            }
        }
        Func::Sign => Ok(Val::number(if x > 0.0 {
            1.0
        } else if x < 0.0 {
            -1.0
        } else {
            0.0
        })),
        _ => Err(ExprError::Value),
    }
}

/// Every value an aggregate's arguments give: a range's values, or an argument's value. `true`
/// marks the values that came from a range (where text and blanks are skipped, as in Excel).
fn each_value(args: &[Expr], env: &dyn Env, f: &mut dyn FnMut(&Val, bool) -> R<()>) -> R<()> {
    for a in args {
        match a {
            Expr::Range(name) => {
                for v in env.range(name)? {
                    f(&v, true)?;
                }
            }
            other => f(&evaluate(other, env), false)?,
        }
    }
    Ok(())
}

/// A number from a range cell: numbers, and text that reads as one (as the report's own totals
/// do); other cells are skipped.
fn cell_number(v: &Val) -> Option<f64> {
    match v {
        Val::Number(n) => Some(*n),
        Val::Text(t) => parse_number(t),
        _ => None,
    }
}

fn aggregate(func: Func, args: &[Expr], env: &dyn Env) -> R<Val> {
    let (mut numbers, mut dates, mut count_all) = (Vec::new(), Vec::new(), 0usize);
    each_value(args, env, &mut |v, from_range| {
        if !matches!(v, Val::Blank) {
            count_all += 1;
        }
        if func == Func::CountA {
            return Ok(());
        }
        if func == Func::Count {
            if !v.is_error() && (cell_number(v).is_some() || matches!(v, Val::Date(_) | Val::DateTime(_))) {
                numbers.push(0.0);
            }
            return Ok(());
        }
        v.err()?;
        if matches!(v, Val::Date(_) | Val::DateTime(_)) || (from_range && cell_number(v).is_none() && v.is_date_like()) {
            dates.extend(v.as_datetime());
        } else if from_range {
            numbers.extend(cell_number(v));
        } else {
            numbers.push(v.to_number()?);
        }
        Ok(())
    })?;
    let count = numbers.len() as f64;
    match func {
        Func::CountA => Ok(Val::Number(count_all as f64)),
        Func::Count => Ok(Val::Number(count)),
        Func::Sum => Ok(Val::number(numbers.iter().sum())),
        Func::Average if count == 0.0 => Err(ExprError::Div0),
        Func::Average => Ok(Val::number(numbers.iter().sum::<f64>() / count)),
        Func::Min | Func::Max if numbers.is_empty() && !dates.is_empty() => {
            let pick = if func == Func::Min { dates.iter().min() } else { dates.iter().max() };
            Ok(pick.map_or(Val::Blank, |d| if d.time() == NaiveTime::MIN { Val::Date(d.date()) } else { Val::DateTime(*d) }))
        }
        Func::Min | Func::Max if numbers.is_empty() => Ok(Val::Number(0.0)),
        Func::Min => Ok(Val::number(numbers.iter().copied().fold(f64::INFINITY, f64::min))),
        Func::Max => Ok(Val::number(numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max))),
        _ => Err(ExprError::Value),
    }
}

/// A `COUNTIF`-style criterion: `"Lyon"`, `">10"`, `"<>"`, `"=a*"`, `">=2026-01-01"`, or a value.
struct Criterion {
    op: Op,
    target: Val,
}

impl Criterion {
    fn read(v: &Val) -> R<Self> {
        v.err()?;
        let Val::Text(t) = v else { return Ok(Self { op: Op::Eq, target: v.clone() }) };
        let (op, rest) = [("<>", Op::Ne), ("<=", Op::Le), (">=", Op::Ge), ("<", Op::Lt), (">", Op::Gt), ("=", Op::Eq)]
            .iter()
            .find_map(|(p, op)| t.strip_prefix(p).map(|rest| (*op, rest)))
            .unwrap_or((Op::Eq, t.as_str()));
        let target = if rest.is_empty() {
            Val::Blank
        } else if let Some(n) = parse_number(rest) {
            Val::Number(n)
        } else if let Some(d) = parse_datetime(rest) {
            Val::DateTime(d)
        } else {
            Val::Text(rest.to_owned())
        };
        Ok(Self { op, target })
    }

    fn matches(&self, cell: &Val) -> bool {
        if cell.is_error() {
            return false;
        }
        let blank = matches!(cell, Val::Blank) || matches!(cell, Val::Text(t) if t.is_empty());
        match (&self.target, self.op) {
            (Val::Blank, Op::Eq) => blank,
            (Val::Blank, Op::Ne) => !blank,
            (Val::Blank, _) => false,
            (Val::Text(pattern), Op::Eq | Op::Ne) => {
                let hit = !matches!(cell, Val::Blank) && wildcard(pattern, &cell.to_string());
                hit == (self.op == Op::Eq)
            }
            (Val::Text(t), op) => matches!(cell, Val::Text(_)) && order_holds(op, compare(cell, &Val::Text(t.clone())).ok()),
            (Val::Number(n), op) => match cell_number(cell) {
                Some(c) => order_holds(op, Some(c.total_cmp(n))),
                None => op == Op::Ne,
            },
            (Val::Date(_) | Val::DateTime(_), op) => match (cell.as_datetime(), self.target.as_datetime()) {
                (Some(c), Some(t)) => order_holds(op, Some(c.cmp(&t))),
                _ => op == Op::Ne,
            },
            (target, op) => order_holds(op, compare(cell, target).ok()),
        }
    }
}

fn order_holds(op: Op, o: Option<Ordering>) -> bool {
    let Some(o) = o else { return false };
    match op {
        Op::Eq => o == Ordering::Equal,
        Op::Ne => o != Ordering::Equal,
        Op::Lt => o == Ordering::Less,
        Op::Gt => o == Ordering::Greater,
        Op::Le => o != Ordering::Greater,
        Op::Ge => o != Ordering::Less,
        _ => false,
    }
}

/// Excel's wildcards, ignoring case: `*` any run, `?` one character, `~` escapes the next one.
fn wildcard(pattern: &str, text: &str) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum P {
        Any,
        One,
        Char(char),
    }
    let mut parts = Vec::new();
    let mut chars = pattern.chars().flat_map(char::to_lowercase);
    while let Some(c) = chars.next() {
        parts.push(match c {
            '*' => P::Any,
            '?' => P::One,
            '~' => P::Char(chars.next().unwrap_or('~')),
            c => P::Char(c),
        });
    }
    let text: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    // can[j]: the pattern's first i parts match the text's first j characters.
    let mut can = vec![false; text.len() + 1];
    if let Some(first) = can.first_mut() {
        *first = true;
    }
    for p in parts {
        let mut next = vec![false; text.len() + 1];
        for j in 0..=text.len() {
            let here = can.get(j).copied().unwrap_or(false);
            let before = j > 0 && can.get(j - 1).copied().unwrap_or(false);
            let prev_next = j > 0 && next.get(j - 1).copied().unwrap_or(false);
            let ch = j.checked_sub(1).and_then(|k| text.get(k));
            let v = match p {
                P::Any => here || prev_next,
                P::One => before,
                P::Char(c) => before && ch == Some(&c),
            };
            if let Some(slot) = next.get_mut(j) {
                *slot = v;
            }
        }
        can = next;
    }
    can.last().copied().unwrap_or(false)
}

fn conditional(func: Func, args: &[Expr], env: &dyn Env) -> R<Val> {
    let range = |i: usize| match args.get(i) {
        Some(Expr::Range(name)) => env.range(name),
        _ => Err(ExprError::Value),
    };
    let cells = range(0)?;
    let criterion = Criterion::read(&args.get(1).map_or(Val::Blank, |a| evaluate(a, env)))?;
    let summed = if args.len() > 2 { range(2)? } else { cells.clone() };
    let (mut total, mut count) = (0.0, 0usize);
    for (i, cell) in cells.iter().enumerate() {
        if !criterion.matches(cell) {
            continue;
        }
        if func == Func::CountIf {
            count += 1;
            continue;
        }
        let v = summed.get(i).unwrap_or(&Val::Blank);
        v.err()?;
        if let Some(n) = cell_number(v) {
            total += n;
            count += 1;
        }
    }
    match func {
        Func::CountIf => Ok(Val::Number(count as f64)),
        Func::SumIf => Ok(Val::number(total)),
        _ if count == 0 => Err(ExprError::Div0),
        _ => Ok(Val::number(total / count as f64)),
    }
}

/// A count argument (`LEFT`'s length, `REPT`'s times): whole, not negative.
fn count(v: &Val) -> R<usize> {
    let n = v.to_number()?.trunc();
    if n < 0.0 { Err(ExprError::Value) } else { Ok(n.min(MAX_TEXT as f64) as usize) }
}

fn text_function(func: Func, a: &[Val]) -> R<Val> {
    let val = |i: usize| a.get(i).cloned().unwrap_or(Val::Blank);
    let s = val(0).to_text()?;
    let chars = || s.chars();
    match func {
        Func::Len => Ok(Val::Number(chars().count() as f64)),
        Func::Left | Func::Right => {
            let n = if a.len() > 1 { count(&val(1))? } else { 1 };
            let total = chars().count();
            let out: String = if func == Func::Left { chars().take(n).collect() } else { chars().skip(total.saturating_sub(n)).collect() };
            Ok(Val::Text(out))
        }
        Func::Mid => {
            let start = val(1).to_number()?.trunc();
            if start < 1.0 {
                return Err(ExprError::Value);
            }
            let start = start.min(MAX_TEXT as f64) as usize - 1;
            Ok(Val::Text(chars().skip(start).take(count(&val(2))?).collect()))
        }
        Func::Upper => Ok(Val::Text(s.to_uppercase())),
        Func::Lower => Ok(Val::Text(s.to_lowercase())),
        Func::Proper => {
            let mut out = String::with_capacity(s.len());
            let mut after_letter = false;
            for c in chars() {
                if after_letter {
                    out.extend(c.to_lowercase())
                } else {
                    out.extend(c.to_uppercase())
                }
                after_letter = c.is_alphabetic();
            }
            Ok(Val::Text(out))
        }
        Func::Trim => Ok(Val::Text(s.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" "))),
        Func::Substitute => substitute(&s, &val(1).to_text()?, &val(2).to_text()?, a.get(3)),
        Func::Find | Func::Search => {
            let within = val(1).to_text()?;
            let start = if a.len() > 2 { count(&val(2))? } else { 1 };
            find(&s, &within, start, func == Func::Search)
        }
        Func::Rept => {
            let n = count(&val(1))?;
            if s.len().saturating_mul(n) > MAX_TEXT {
                return Err(ExprError::Value);
            }
            text(s.repeat(n))
        }
        Func::Exact => Ok(Val::Bool(s == val(1).to_text()?)),
        Func::Value => match val(0) {
            Val::Number(n) => Ok(Val::Number(n)),
            Val::Blank => Ok(Val::Number(0.0)),
            _ => parse_number(&s).map(Val::Number).ok_or(ExprError::Value),
        },
        _ => Err(ExprError::Value),
    }
}

fn substitute(s: &str, old: &str, new: &str, instance: Option<&Val>) -> R<Val> {
    if old.is_empty() {
        return Ok(Val::Text(s.to_owned()));
    }
    let out = match instance {
        None => s.replace(old, new),
        Some(v) => {
            let n = count(v)?;
            if n == 0 {
                return Err(ExprError::Value);
            }
            match s.match_indices(old).nth(n - 1) {
                Some((at, _)) => format!("{}{new}{}", s.get(..at).unwrap_or(""), s.get(at + old.len()..).unwrap_or("")),
                None => s.to_owned(),
            }
        }
    };
    text(out)
}

/// 1-based position of `needle` in `hay` from character `start`.
fn find(needle: &str, hay: &str, start: usize, ignore_case: bool) -> R<Val> {
    let norm = |c: char| if ignore_case { c.to_lowercase().next().unwrap_or(c) } else { c };
    let hay: Vec<char> = hay.chars().map(norm).collect();
    let needle: Vec<char> = needle.chars().map(norm).collect();
    if start == 0 || start > hay.len() + 1 {
        return Err(ExprError::Value);
    }
    if needle.is_empty() {
        return Ok(Val::Number(start as f64));
    }
    let from = start - 1;
    hay.get(from..)
        .and_then(|rest| rest.windows(needle.len()).position(|w| w == needle.as_slice()))
        .map(|p| Val::Number((from + p + 1) as f64))
        .ok_or(ExprError::Value)
}

/// A whole number argument, bounded.
fn whole(v: &Val, limit: f64) -> R<i64> {
    let n = v.to_number()?.trunc();
    if n.abs() > limit { Err(ExprError::Num) } else { Ok(n as i64) }
}

fn add_months(d: NaiveDate, months: i64) -> R<NaiveDate> {
    let m = Months::new(u32::try_from(months.unsigned_abs()).map_err(|_| ExprError::Num)?);
    if months >= 0 { d.checked_add_months(m) } else { d.checked_sub_months(m) }.ok_or(ExprError::Num)
}

fn date_function(func: Func, a: &[Val]) -> R<Val> {
    let val = |i: usize| a.get(i).cloned().unwrap_or(Val::Blank);
    if func == Func::Date {
        let mut year = whole(&val(0), 10_000.0)?;
        if (0..1900).contains(&year) {
            year += 1900;
        }
        if !(0..=9999).contains(&year) {
            return Err(ExprError::Num);
        }
        let first = NaiveDate::from_ymd_opt(i32::try_from(year).map_err(|_| ExprError::Num)?, 1, 1).ok_or(ExprError::Num)?;
        let month = add_months(first, whole(&val(1), 1_000_000.0)? - 1)?;
        let day = month.checked_add_signed(TimeDelta::try_days(whole(&val(2), 10_000_000.0)? - 1).ok_or(ExprError::Num)?).ok_or(ExprError::Num)?;
        return Ok(Val::Date(day));
    }
    // An empty date stays empty.
    let dates_given = if matches!(func, Func::Days | Func::DateDif) { 2 } else { 1 };
    if a.iter().take(dates_given).any(|v| matches!(v, Val::Blank)) {
        return Ok(Val::Blank);
    }
    let dt = || {
        let v = val(0);
        v.err()?;
        v.as_datetime().ok_or(ExprError::Value)
    };
    match func {
        Func::DateValue => Ok(Val::Date(dt()?.date())),
        Func::Year => Ok(Val::Number(f64::from(dt()?.year()))),
        Func::Month => Ok(Val::Number(f64::from(dt()?.month()))),
        Func::Day => Ok(Val::Number(f64::from(dt()?.day()))),
        Func::Hour => Ok(Val::Number(f64::from(dt()?.hour()))),
        Func::Minute => Ok(Val::Number(f64::from(dt()?.minute()))),
        Func::Weekday => {
            let wd = dt()?.weekday();
            let kind = if a.len() > 1 { whole(&val(1), 100.0)? } else { 1 };
            let n = match kind {
                1 => wd.num_days_from_sunday() + 1,
                2 => wd.number_from_monday(),
                3 => wd.num_days_from_monday(),
                _ => return Err(ExprError::Num),
            };
            Ok(Val::Number(f64::from(n)))
        }
        Func::IsoWeekNum => Ok(Val::Number(f64::from(dt()?.iso_week().week()))),
        Func::EDate => Ok(Val::Date(add_months(dt()?.date(), whole(&val(1), 1_000_000.0)?)?)),
        Func::EoMonth => {
            let d = add_months(dt()?.date(), whole(&val(1), 1_000_000.0)?)?;
            let first = d.with_day(1).ok_or(ExprError::Num)?;
            let last = add_months(first, 1)?.pred_opt().ok_or(ExprError::Num)?;
            Ok(Val::Date(last))
        }
        Func::Days => Ok(Val::Number((val(0).to_date()? - val(1).to_date()?).num_days() as f64)),
        Func::DateDif => {
            let (start, end) = (val(0).to_date()?, val(1).to_date()?);
            if start > end {
                return Err(ExprError::Num);
            }
            let months = {
                let m = (end.year() - start.year()) * 12 + end.month() as i32 - start.month() as i32;
                if end.day() < start.day() { m - 1 } else { m }
            };
            match val(2).to_text()?.to_ascii_uppercase().as_str() {
                "Y" => Ok(Val::Number(f64::from(months / 12))),
                "M" => Ok(Val::Number(f64::from(months))),
                "D" => Ok(Val::Number((end - start).num_days() as f64)),
                _ => Err(ExprError::Num),
            }
        }
        _ => Err(ExprError::Value),
    }
}
