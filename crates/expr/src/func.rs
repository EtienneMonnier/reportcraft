//! The function catalogue: names, argument counts and which arguments are ranges.

/// A function a formula may call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Func {
    // Logical
    If,
    Ifs,
    Switch,
    And,
    Or,
    Not,
    IfError,
    IsBlank,
    IsError,
    IsNumber,
    IsText,
    // Maths
    Abs,
    Round,
    RoundUp,
    RoundDown,
    Int,
    Mod,
    Power,
    Sqrt,
    Sign,
    // Aggregates (a field argument is a range)
    Sum,
    Average,
    Count,
    CountA,
    Min,
    Max,
    SumIf,
    CountIf,
    AverageIf,
    // Text
    Concat,
    Len,
    Left,
    Right,
    Mid,
    Upper,
    Lower,
    Proper,
    Trim,
    Substitute,
    Find,
    Search,
    Rept,
    Exact,
    Value,
    Text,
    // Dates
    Today,
    Now,
    Date,
    DateValue,
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Weekday,
    IsoWeekNum,
    EDate,
    EoMonth,
    Days,
    DateDif,
    // Report
    Param,
}

/// Name, function, fewest and most arguments.
const CATALOGUE: &[(&str, Func, usize, usize)] = &[
    ("IF", Func::If, 2, 3),
    ("IFS", Func::Ifs, 2, crate::MAX_ARGS),
    ("SWITCH", Func::Switch, 3, crate::MAX_ARGS),
    ("AND", Func::And, 1, crate::MAX_ARGS),
    ("OR", Func::Or, 1, crate::MAX_ARGS),
    ("NOT", Func::Not, 1, 1),
    ("IFERROR", Func::IfError, 2, 2),
    ("ISBLANK", Func::IsBlank, 1, 1),
    ("ISERROR", Func::IsError, 1, 1),
    ("ISNUMBER", Func::IsNumber, 1, 1),
    ("ISTEXT", Func::IsText, 1, 1),
    ("ABS", Func::Abs, 1, 1),
    ("ROUND", Func::Round, 2, 2),
    ("ROUNDUP", Func::RoundUp, 2, 2),
    ("ROUNDDOWN", Func::RoundDown, 2, 2),
    ("INT", Func::Int, 1, 1),
    ("MOD", Func::Mod, 2, 2),
    ("POWER", Func::Power, 2, 2),
    ("SQRT", Func::Sqrt, 1, 1),
    ("SIGN", Func::Sign, 1, 1),
    ("SUM", Func::Sum, 1, crate::MAX_ARGS),
    ("AVERAGE", Func::Average, 1, crate::MAX_ARGS),
    ("COUNT", Func::Count, 1, crate::MAX_ARGS),
    ("COUNTA", Func::CountA, 1, crate::MAX_ARGS),
    ("MIN", Func::Min, 1, crate::MAX_ARGS),
    ("MAX", Func::Max, 1, crate::MAX_ARGS),
    ("SUMIF", Func::SumIf, 2, 3),
    ("COUNTIF", Func::CountIf, 2, 2),
    ("AVERAGEIF", Func::AverageIf, 2, 3),
    ("CONCAT", Func::Concat, 1, crate::MAX_ARGS),
    ("CONCATENATE", Func::Concat, 1, crate::MAX_ARGS),
    ("LEN", Func::Len, 1, 1),
    ("LEFT", Func::Left, 1, 2),
    ("RIGHT", Func::Right, 1, 2),
    ("MID", Func::Mid, 3, 3),
    ("UPPER", Func::Upper, 1, 1),
    ("LOWER", Func::Lower, 1, 1),
    ("PROPER", Func::Proper, 1, 1),
    ("TRIM", Func::Trim, 1, 1),
    ("SUBSTITUTE", Func::Substitute, 3, 4),
    ("FIND", Func::Find, 2, 3),
    ("SEARCH", Func::Search, 2, 3),
    ("REPT", Func::Rept, 2, 2),
    ("EXACT", Func::Exact, 2, 2),
    ("VALUE", Func::Value, 1, 1),
    ("TEXT", Func::Text, 2, 2),
    ("TODAY", Func::Today, 0, 0),
    ("NOW", Func::Now, 0, 0),
    ("DATE", Func::Date, 3, 3),
    ("DATEVALUE", Func::DateValue, 1, 1),
    ("YEAR", Func::Year, 1, 1),
    ("MONTH", Func::Month, 1, 1),
    ("DAY", Func::Day, 1, 1),
    ("HOUR", Func::Hour, 1, 1),
    ("MINUTE", Func::Minute, 1, 1),
    ("WEEKDAY", Func::Weekday, 1, 2),
    ("ISOWEEKNUM", Func::IsoWeekNum, 1, 1),
    ("EDATE", Func::EDate, 2, 2),
    ("EOMONTH", Func::EoMonth, 2, 2),
    ("DAYS", Func::Days, 2, 2),
    ("DATEDIF", Func::DateDif, 3, 3),
    ("PARAM", Func::Param, 1, 1),
];

impl Func {
    /// The function called `name` (any case).
    pub fn lookup(name: &str) -> Option<Func> {
        CATALOGUE.iter().find(|(n, ..)| n.eq_ignore_ascii_case(name)).map(|&(_, f, ..)| f)
    }

    /// Fewest and most arguments.
    pub fn arity(self) -> (usize, usize) {
        CATALOGUE.iter().find(|(_, f, ..)| *f == self).map_or((0, 0), |&(_, _, min, max)| (min, max))
    }

    /// The name formulas use.
    pub fn name(self) -> &'static str {
        CATALOGUE.iter().find(|(_, f, ..)| *f == self).map_or("", |&(n, ..)| n)
    }

    /// Every function name, for documentation and tools.
    pub fn names() -> impl Iterator<Item = &'static str> {
        CATALOGUE.iter().map(|(n, ..)| *n)
    }

    /// Whether argument `index` is a range when it is a field: the field over the rows being
    /// summarised rather than the current row's value.
    pub fn takes_range(self, index: usize) -> bool {
        match self {
            Func::Sum | Func::Average | Func::Count | Func::CountA | Func::Min | Func::Max => true,
            Func::SumIf | Func::AverageIf => index == 0 || index == 2,
            Func::CountIf => index == 0,
            _ => false,
        }
    }
}
