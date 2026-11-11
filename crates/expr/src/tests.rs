use std::cell::Cell;

use chrono::{NaiveDate, NaiveDateTime};

use crate::{Env, ExprError, Val, evaluate, parse};

/// One current row and the rows of its group, as `(field, values)`.
struct Rows {
    current: usize,
    columns: Vec<(&'static str, Vec<Val>)>,
    now: Option<NaiveDateTime>,
    computed: Cell<usize>,
}

impl Env for Rows {
    fn field(&self, name: &str) -> Val {
        match self.columns.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            Some((_, values)) => values.get(self.current).cloned().unwrap_or_default(),
            None => Val::Error(ExprError::Name),
        }
    }

    fn range(&self, name: &str) -> Result<Vec<Val>, ExprError> {
        self.computed.set(self.computed.get() + 1);
        self.columns.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone()).ok_or(ExprError::Name)
    }

    fn param(&self, name: &str) -> Val {
        if name == "min" { Val::Number(5.0) } else { Val::Blank }
    }

    fn now(&self) -> Option<NaiveDateTime> {
        self.now
    }
}

fn rows() -> Rows {
    let t = |s: &str| Val::Text(s.into());
    Rows {
        current: 0,
        columns: vec![
            ("branch", vec![t("Lyon"), t("Paris"), t("Lyon"), t("lyon-sud")]),
            ("amount", vec![Val::Number(10.0), Val::Number(5.0), Val::Number(2.5), Val::Blank]),
            ("start", vec![t("2026-01-31"), t("2026-03-01"), t("2026-02-15 08:30:00"), Val::Blank]),
            ("note", vec![Val::Blank, t("x"), t("12"), t("")]),
        ],
        now: NaiveDate::from_ymd_opt(2026, 11, 11).and_then(|d| d.and_hms_opt(9, 30, 0)),
        computed: Cell::new(0),
    }
}

fn eval(formula: &str) -> Val {
    evaluate(&parse(formula).unwrap_or_else(|e| panic!("{formula}: {e}")), &rows())
}

fn show(formula: &str) -> String {
    eval(formula).to_string()
}

#[test]
fn arithmetic_and_precedence() {
    assert_eq!(show("=1+2*3"), "7");
    assert_eq!(show("(1+2)*3"), "9");
    assert_eq!(show("-2^2"), "4");
    assert_eq!(show("2^3^2"), "64");
    assert_eq!(show("50%"), "0.5");
    assert_eq!(show("0.1+0.2"), "0.3");
    assert_eq!(show("1/3"), "0.333333333333333");
    assert_eq!(show("\"3\" + 4"), "7");
    assert_eq!(show("TRUE + 1"), "2");
    assert_eq!(show("[note] + 1"), "1", "a blank is 0");
}

#[test]
fn errors_are_values_until_caught() {
    assert_eq!(eval("1/0"), Val::Error(ExprError::Div0));
    assert_eq!(eval("\"a\" * 2"), Val::Error(ExprError::Value));
    assert_eq!(eval("[nope] + 1"), Val::Error(ExprError::Name));
    assert_eq!(eval("SQRT(-1)"), Val::Error(ExprError::Num));
    assert_eq!(eval("10^400"), Val::Error(ExprError::Num));
    assert_eq!(show("IFERROR(1/0, \"none\")"), "none");
    assert_eq!(show("ISERROR(1/0)"), "TRUE");
    assert_eq!(show("(1/0) & \"x\""), "#DIV/0!", "the first error wins");
    assert_eq!(eval("0^0"), Val::Error(ExprError::Num));
    assert_eq!(eval("0^-1"), Val::Error(ExprError::Div0));
}

#[test]
fn comparisons_follow_excel() {
    assert_eq!(show("\"abc\" = \"ABC\""), "TRUE");
    assert_eq!(show("1 < \"0\""), "TRUE", "numbers sort before text");
    assert_eq!(show("[note] = 0"), "TRUE");
    assert_eq!(show("[note] = \"\""), "TRUE");
    assert_eq!(show("[start] < DATE(2026,2,1)"), "TRUE", "text holding a date compares as a date");
    assert_eq!(show("[branch] & \"!\" <> \"Lyon\""), "TRUE");
}

#[test]
fn logical_functions_evaluate_only_what_they_need() {
    assert_eq!(show("IF([amount] > 5, \"big\", \"small\")"), "big");
    assert_eq!(show("IF(FALSE, 1)"), "FALSE");
    assert_eq!(show("IF(TRUE, 1, 1/0)"), "1");
    assert_eq!(show("IFS([amount] > 50, \"a\", [amount] > 5, \"b\")"), "b");
    assert_eq!(eval("IFS(FALSE, 1)"), Val::Error(ExprError::Na));
    assert_eq!(show("SWITCH([branch], \"Paris\", 1, \"lyon\", 2, 0)"), "2");
    assert_eq!(show("SWITCH(3, 1, \"a\", \"other\")"), "other");
    assert_eq!(show("AND(TRUE, 1, NOT(FALSE))"), "TRUE");
    assert_eq!(show("OR(FALSE, 0)"), "FALSE");
    assert_eq!(eval("AND(\"x\")"), Val::Error(ExprError::Value));
}

#[test]
fn rounding_matches_what_is_written() {
    assert_eq!(show("ROUND(2.675, 2)"), "2.68");
    assert_eq!(show("ROUND(-2.5, 0)"), "-3");
    assert_eq!(show("ROUND(1234.5, -2)"), "1200");
    assert_eq!(show("ROUNDUP(1.201, 2)"), "1.21");
    assert_eq!(show("ROUNDUP(-1.201, 2)"), "-1.21");
    assert_eq!(show("ROUNDDOWN(1.299, 1)"), "1.2");
    assert_eq!(show("INT(-1.5)"), "-2");
    assert_eq!(show("MOD(-3, 2)"), "1");
    assert_eq!(eval("MOD(1, 0)"), Val::Error(ExprError::Div0));
    assert_eq!(show("POWER(2, 10) + ABS(-1) + SIGN(-4)"), "1024");
}

#[test]
fn aggregates_summarise_ranges() {
    assert_eq!(show("SUM([amount])"), "17.5");
    assert_eq!(show("AVERAGE([amount])"), "5.83333333333333");
    assert_eq!(show("COUNT([amount])"), "3");
    assert_eq!(show("COUNTA([note])"), "3", "empty text is not blank, as in Excel");
    assert_eq!(show("COUNT([note])"), "1", "numeric text counts, as the report's totals do");
    assert_eq!(show("MIN([amount]) & \"-\" & MAX([amount])"), "2.5-10");
    assert_eq!(show("MAX([start])"), "2026-03-01", "dates have a latest");
    assert_eq!(show("SUM([amount], 1, \"2\")"), "20.5");
    assert_eq!(show("[amount] / SUM([amount])"), "0.571428571428571");
    assert_eq!(show("SUM([branch])"), "0", "text in a range is skipped");
    assert_eq!(eval("SUM(\"x\")"), Val::Error(ExprError::Value), "but not given directly");
    assert_eq!(eval("SUM([nope])"), Val::Error(ExprError::Name));
}

#[test]
fn conditional_aggregates_read_criteria() {
    assert_eq!(show("COUNTIF([branch], \"lyon\")"), "2");
    assert_eq!(show("COUNTIF([branch], \"lyon*\")"), "3");
    assert_eq!(show("COUNTIF([branch], \"<>Lyon\")"), "2");
    assert_eq!(show("SUMIF([branch], \"Lyon\", [amount])"), "12.5");
    assert_eq!(show("SUMIF([amount], \">=5\")"), "15");
    assert_eq!(show("AVERAGEIF([amount], \">\" & PARAM(\"min\"))"), "10");
    assert_eq!(show("COUNTIF([note], \"\")"), "2", "blank and empty text");
    assert_eq!(show("COUNTIF([note], \"<>\")"), "2");
    assert_eq!(show("COUNTIF([start], \">=2026-02-01\")"), "2");
    assert_eq!(show("COUNTIF([amount], 5)"), "1");
    assert_eq!(show("COUNTIF([branch], \"P?ris\")"), "1");
    assert_eq!(eval("AVERAGEIF([amount], \">100\")"), Val::Error(ExprError::Div0));
    assert_eq!(eval("SUMIF(1, 1)"), Val::Error(ExprError::Value), "the first argument must be a field");
}

#[test]
fn row_independent_aggregates_are_remembered() {
    struct Memo(Rows, std::cell::RefCell<std::collections::HashMap<usize, Val>>);
    impl Env for Memo {
        fn field(&self, name: &str) -> Val {
            self.0.field(name)
        }
        fn range(&self, name: &str) -> Result<Vec<Val>, ExprError> {
            self.0.range(name)
        }
        fn param(&self, name: &str) -> Val {
            self.0.param(name)
        }
        fn now(&self) -> Option<NaiveDateTime> {
            None
        }
        fn memo(&self, key: usize, compute: &mut dyn FnMut() -> Val) -> Val {
            if let Some(v) = self.1.borrow().get(&key) {
                return v.clone();
            }
            let v = compute();
            self.1.borrow_mut().insert(key, v.clone());
            v
        }
    }
    let env = Memo(rows(), Default::default());
    let e = parse("[amount] / SUM([amount]) + SUMIF([branch], [branch], [amount])").unwrap();
    for _ in 0..3 {
        evaluate(&e, &env);
    }
    // SUM is computed once; SUMIF reads the current row, so it runs every time (2 ranges each).
    assert_eq!(env.0.computed.get(), 1 + 3 * 2);
}

#[test]
fn text_functions_count_characters() {
    assert_eq!(show("CONCAT([branch], \" – \", 3)"), "Lyon – 3");
    assert_eq!(show("CONCATENATE(\"a\", TRUE)"), "aTRUE");
    assert_eq!(show("LEN(\"Évry\")"), "4");
    assert_eq!(show("LEFT(\"Évry\", 2) & RIGHT(\"Évry\") & MID(\"Évry\", 2, 2)"), "Évyvr");
    assert_eq!(show("UPPER(\"é\") & LOWER(\"ÉA\") & PROPER(\"jean-luc o'neil\")"), "ÉéaJean-Luc O'Neil");
    assert_eq!(show("TRIM(\"  a   b  \")"), "a b");
    assert_eq!(show("SUBSTITUTE(\"a-b-c\", \"-\", \"+\")"), "a+b+c");
    assert_eq!(show("SUBSTITUTE(\"a-b-c\", \"-\", \"+\", 2)"), "a-b+c");
    assert_eq!(show("FIND(\"b\", \"abcb\", 3)"), "4");
    assert_eq!(eval("FIND(\"B\", \"abc\")"), Val::Error(ExprError::Value));
    assert_eq!(show("SEARCH(\"B\", \"abc\")"), "2");
    assert_eq!(show("REPT(\"ab\", 3)"), "ababab");
    assert_eq!(eval("REPT(\"ab\", 1e9)"), Val::Error(ExprError::Value));
    assert_eq!(show("EXACT(\"a\", \"A\")"), "FALSE");
    assert_eq!(show("VALUE(\" 12.5 \") * 2"), "25");
    assert_eq!(eval("MID(\"abc\", 0, 1)"), Val::Error(ExprError::Value));
    assert_eq!(eval("LEFT(\"abc\", -1)"), Val::Error(ExprError::Value));
    let long = format!("REPT(\"x\", {}) & \"y\"", crate::MAX_TEXT);
    assert_eq!(eval(&long), Val::Error(ExprError::Value));
}

#[test]
fn dates_are_values_of_their_own() {
    assert_eq!(show("DATE(2026, 13, 1)"), "2027-01-01");
    assert_eq!(show("DATE(2026, 3, 0)"), "2026-02-28");
    assert_eq!(show("DATE(26, 1, 1)"), "1926-01-01", "Excel adds 1900 to years below 1900");
    assert_eq!(show("[start] + 30"), "2026-03-02");
    assert_eq!(show("DATE(2026,3,1) - [start]"), "29");
    assert_eq!(show("[start] - 1"), "2026-01-30");
    assert_eq!(show("DATE(2026,1,1) + 0.5"), "2026-01-01 12:00:00");
    assert_eq!(eval("DATE(2026,1,1) + DATE(2026,1,1)"), Val::Error(ExprError::Value));
    assert_eq!(eval("DATE(2026,1,1) * 2"), Val::Error(ExprError::Value));
    assert_eq!(show("YEAR([start]) & \"/\" & MONTH([start]) & \"/\" & DAY([start])"), "2026/1/31");
    assert_eq!(show("WEEKDAY(DATE(2026,11,11))"), "4");
    assert_eq!(show("WEEKDAY(DATE(2026,11,11), 2)"), "3");
    assert_eq!(show("ISOWEEKNUM(DATE(2026,1,1))"), "1");
    assert_eq!(show("EDATE([start], 1)"), "2026-02-28");
    assert_eq!(show("EOMONTH(DATE(2024,1,15), 1)"), "2024-02-29");
    assert_eq!(show("DAYS(DATE(2026,3,1), [start])"), "29");
    assert_eq!(show("DATEDIF(DATE(2020,2,29), DATE(2026,2,28), \"Y\")"), "5");
    assert_eq!(show("DATEDIF([start], DATE(2026,3,30), \"M\")"), "1");
    assert_eq!(eval("DATEDIF(DATE(2026,2,1), DATE(2026,1,1), \"D\")"), Val::Error(ExprError::Num));
    assert_eq!(show("DATEVALUE(\"2026-10-31T14:05:09+02:00\")"), "2026-10-31");
    assert_eq!(show("HOUR(\"2026-02-15 08:30:00\") * 60 + MINUTE(\"2026-02-15 08:30:00\")"), "510");
    assert_eq!(eval("YEAR(\"soon\")"), Val::Error(ExprError::Value));
    assert_eq!(eval("YEAR(45000)"), Val::Error(ExprError::Value), "no serial numbers");
    assert_eq!(eval("DATE(2026, 1e9, 1)"), Val::Error(ExprError::Num));
    assert_eq!(eval("DATE(10000, 1, 1)"), Val::Error(ExprError::Num));
    assert_eq!(eval("[start] + 1e300"), Val::Error(ExprError::Num));
}

#[test]
fn an_empty_date_stays_empty() {
    let mut r = rows();
    r.current = 3;
    for f in ["YEAR([start])", "EDATE([start], 1)", "DAYS([start], DATE(2026,1,1))", "DATEVALUE([start])"] {
        assert_eq!(evaluate(&parse(f).unwrap(), &r), Val::Blank, "{f}");
    }
}

#[test]
fn clock_and_parameters_come_from_the_render() {
    assert_eq!(show("TODAY()"), "2026-11-11");
    assert_eq!(show("NOW()"), "2026-11-11 09:30:00");
    assert_eq!(show("TODAY() - DATE(2026,11,1)"), "10");
    assert_eq!(show("PARAM(\"min\") * 2"), "10");
    assert_eq!(eval("PARAM(\"missing\")"), Val::Blank);
    let mut r = rows();
    r.now = None;
    assert_eq!(evaluate(&parse("TODAY()").unwrap(), &r), Val::Error(ExprError::Na));
}

#[test]
fn field_names_may_carry_their_table() {
    let names: Vec<&str> = crate::field_candidates("order.date").collect();
    assert_eq!(names, ["order.date", "date"]);
    assert_eq!(crate::field_candidates("amount").count(), 1);
    assert_eq!(crate::field_candidates("odd.").count(), 1);
}
