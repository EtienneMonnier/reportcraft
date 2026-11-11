//! ReportCraft formulas (L0): Excel-style expressions, parsed once and evaluated per row or per
//! group. No I/O, no clock, no dependencies on other ReportCraft crates, wasm-safe.
//!
//! Syntax: an optional leading `=`, numbers (`1.5`, `2e3`), text in double quotes (`"a ""b"""`),
//! `TRUE`/`FALSE`, fields in brackets (`[amount]`, `[order.date]`), the operators
//! `+ - * / ^ % &` and `= <> < > <= >=`, and function calls whose arguments are separated by `,`
//! or `;`. Function names are case-insensitive. Precedence follows Excel: negation, `%`, `^`,
//! `* /`, `+ -`, `&`, comparisons (so `-2^2` is 4).
//!
//! Semantics follow Excel where it has an answer: a blank is 0 in arithmetic and `""` in text,
//! errors (`#DIV/0!`, `#VALUE!`, `#NAME?`, `#NUM!`, `#N/A`) are values that flow through
//! calculations until `IFERROR` catches them, and a field given directly to `SUM`, `AVERAGE`,
//! `COUNT`, `COUNTA`, `MIN`, `MAX`, `SUMIF`, `COUNTIF` or `AVERAGEIF` is a range: that field over
//! the rows being summarised. Dates are values of their own (Excel's serial numbers aren't
//! exposed): a date plus a number of days is a date, a date minus a date is a number of days,
//! and text holding an ISO date (`2026-10-31`, as data sources write dates) is read as a date
//! where a date is expected.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod eval;
mod func;
mod parse;
mod value;

#[cfg(test)]
mod tests;

pub use eval::{Env, evaluate};
pub use func::Func;
pub use parse::{Expr, ParseError, parse};
pub use value::{ExprError, Val, significant};

/// Longest formula accepted, in bytes.
pub const MAX_LEN: usize = 8_192;
/// Deepest nesting of operators and calls accepted (also bounds evaluation recursion).
pub const MAX_DEPTH: usize = 100;
/// Most arguments one call may take (Excel's limit).
pub const MAX_ARGS: usize = 255;
/// Longest text a formula may produce, in bytes (`REPT` and `&` stop there with `#VALUE!`).
pub const MAX_TEXT: usize = 32_768;

/// The column names a field reference may mean, most specific first: `[order.date]` is a column
/// named `order.date`, else `date` (a field written with its table's name).
pub fn field_candidates(name: &str) -> impl Iterator<Item = &str> {
    let short = name.rsplit_once('.').map(|(_, f)| f).filter(|f| !f.is_empty());
    std::iter::once(name).chain(short)
}
