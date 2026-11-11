//! Formula text to a syntax tree, with its length and depth bounded.

use std::fmt;

use crate::Func;

/// A parsed formula.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Text(String),
    Bool(bool),
    /// The current row's value of a field.
    Field(String),
    /// A field over the rows being summarised (a field given directly to an aggregate).
    Range(String),
    Neg(Box<Expr>),
    /// `x%`: x divided by 100.
    Percent(Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
    Call {
        func: Func,
        args: Vec<Expr>,
        /// No argument reads the current row: the result is the same for every row of a scope.
        row_independent: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

/// Why a formula doesn't parse, and where (a character offset into the text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub position: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at character {})", self.message, self.position + 1)
    }
}

impl std::error::Error for ParseError {}

impl Expr {
    /// Every field the formula names, in order, ranges included (each name once).
    pub fn fields(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        self.visit(&mut |e| {
            if let Expr::Field(n) | Expr::Range(n) = e
                && !out.iter().any(|o| o.eq_ignore_ascii_case(n))
            {
                out.push(n);
            }
        });
        out
    }

    /// The parameter names given as fixed text to `PARAM`.
    pub fn params(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.visit(&mut |e| {
            if let Expr::Call { func: Func::Param, args, .. } = e
                && let Some(Expr::Text(name)) = args.first()
            {
                out.push(name.as_str());
            }
        });
        out
    }

    /// Whether the formula summarises rows (a field given to an aggregate).
    pub fn uses_ranges(&self) -> bool {
        let mut found = false;
        self.visit(&mut |e| found |= matches!(e, Expr::Range(_)));
        found
    }

    /// Whether the formula reads the current row.
    pub fn uses_row(&self) -> bool {
        let mut found = false;
        self.visit(&mut |e| found |= matches!(e, Expr::Field(_)));
        found
    }

    fn visit<'a>(&'a self, f: &mut dyn FnMut(&'a Expr)) {
        f(self);
        match self {
            Expr::Neg(a) | Expr::Percent(a) => a.visit(f),
            Expr::Binary(_, a, b) => {
                a.visit(f);
                b.visit(f);
            }
            Expr::Call { args, .. } => args.iter().for_each(|a| a.visit(f)),
            Expr::Number(_) | Expr::Text(_) | Expr::Bool(_) | Expr::Field(_) | Expr::Range(_) => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Number(f64),
    Text(String),
    Field(String),
    Name(String),
    Open,
    Close,
    Sep,
    Op(Op),
    Plus,
    Minus,
    Percent,
}

fn err<T>(position: usize, message: impl Into<String>) -> Result<T, ParseError> {
    Err(ParseError { position, message: message.into() })
}

/// Reads a formula (`=` first or not).
pub fn parse(text: &str) -> Result<Expr, ParseError> {
    if text.len() > crate::MAX_LEN {
        return err(0, format!("formula longer than {} characters", crate::MAX_LEN));
    }
    let tokens = lex(text)?;
    let mut p = Parser { tokens, pos: 0, end: text.chars().count(), level: 0 };
    let (expr, _) = p.comparison()?;
    match p.tokens.get(p.pos) {
        None => Ok(expr),
        Some((at, Tok::Close)) => err(*at, "`)` without a matching `(`"),
        Some((at, _)) => err(*at, "unexpected text after the formula (an operator or `,` missing?)"),
    }
}

fn lex(text: &str) -> Result<Vec<(usize, Tok)>, ParseError> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    // One leading `=`, as in a spreadsheet cell.
    while chars.get(i).is_some_and(|c| c.is_whitespace()) {
        i += 1;
    }
    if chars.get(i) == Some(&'=') {
        i += 1;
    }
    while let Some(&c) = chars.get(i) {
        let start = i;
        i += 1;
        let tok = match c {
            c if c.is_whitespace() => continue,
            '(' => Tok::Open,
            ')' => Tok::Close,
            ',' | ';' => Tok::Sep,
            '+' => Tok::Plus,
            '-' => Tok::Minus,
            '%' => Tok::Percent,
            '*' => Tok::Op(Op::Mul),
            '/' => Tok::Op(Op::Div),
            '^' => Tok::Op(Op::Pow),
            '&' => Tok::Op(Op::Concat),
            '=' => Tok::Op(Op::Eq),
            '<' | '>' => {
                let next = chars.get(i).copied();
                let op = match (c, next) {
                    ('<', Some('>')) => Some(Op::Ne),
                    ('<', Some('=')) => Some(Op::Le),
                    ('>', Some('=')) => Some(Op::Ge),
                    _ => None,
                };
                match op {
                    Some(op) => {
                        i += 1;
                        Tok::Op(op)
                    }
                    None => Tok::Op(if c == '<' { Op::Lt } else { Op::Gt }),
                }
            }
            '"' => {
                let mut s = String::new();
                loop {
                    match chars.get(i) {
                        None => return err(start, "text without its closing `\"`"),
                        Some('"') if chars.get(i + 1) == Some(&'"') => {
                            s.push('"');
                            i += 2;
                        }
                        Some('"') => {
                            i += 1;
                            break;
                        }
                        Some(&ch) => {
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                Tok::Text(s)
            }
            '[' => {
                let close = chars.iter().skip(i).position(|&ch| ch == ']').map(|n| i + n);
                let Some(close) = close else { return err(start, "field without its closing `]`") };
                let name: String = chars.get(i..close).unwrap_or_default().iter().collect();
                let name = name.trim().to_owned();
                if name.is_empty() || name.contains('[') {
                    return err(start, "a field name goes between `[` and `]`");
                }
                i = close + 1;
                Tok::Field(name)
            }
            c if c.is_ascii_digit() || c == '.' => {
                let mut j = i;
                while chars.get(j).is_some_and(|ch| ch.is_ascii_digit() || *ch == '.') {
                    j += 1;
                }
                if matches!(chars.get(j), Some('e' | 'E')) {
                    let mut k = j + 1;
                    if matches!(chars.get(k), Some('+' | '-')) {
                        k += 1;
                    }
                    if chars.get(k).is_some_and(char::is_ascii_digit) {
                        j = k;
                        while chars.get(j).is_some_and(char::is_ascii_digit) {
                            j += 1;
                        }
                    }
                }
                let s: String = chars.get(start..j).unwrap_or_default().iter().collect();
                i = j;
                match s.parse::<f64>() {
                    Ok(n) if n.is_finite() => Tok::Number(n),
                    _ => return err(start, format!("`{s}` is not a number")),
                }
            }
            c if c.is_alphabetic() || c == '_' => {
                let mut j = i;
                while chars.get(j).is_some_and(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '.') {
                    j += 1;
                }
                let s: String = chars.get(start..j).unwrap_or_default().iter().collect();
                i = j;
                Tok::Name(s)
            }
            other => return err(start, format!("unexpected `{other}`")),
        };
        out.push((start, tok));
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<(usize, Tok)>,
    pos: usize,
    /// Character count of the text, for errors at its end.
    end: usize,
    /// Brackets and calls open at this point.
    level: usize,
}

/// A node and the depth of its tree.
type Node = (Expr, usize);

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos).map(|(_, t)| t)
    }

    fn at(&self) -> usize {
        self.tokens.get(self.pos).map_or(self.end, |(p, _)| *p)
    }

    fn binary(&self, op: Op, (a, da): Node, (b, db): Node) -> Result<Node, ParseError> {
        let depth = da.max(db) + 1;
        if depth > crate::MAX_DEPTH {
            return err(self.at(), format!("formula nested more than {} deep", crate::MAX_DEPTH));
        }
        Ok((Expr::Binary(op, Box::new(a), Box::new(b)), depth))
    }

    fn left_assoc(&mut self, ops: &[Op], next: fn(&mut Self) -> Result<Node, ParseError>) -> Result<Node, ParseError> {
        let mut left = next(self)?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op(op)) if ops.contains(op) => *op,
                Some(Tok::Plus) if ops.contains(&Op::Add) => Op::Add,
                Some(Tok::Minus) if ops.contains(&Op::Sub) => Op::Sub,
                _ => return Ok(left),
            };
            self.pos += 1;
            let right = next(self)?;
            left = self.binary(op, left, right)?;
        }
    }

    fn comparison(&mut self) -> Result<Node, ParseError> {
        self.left_assoc(&[Op::Eq, Op::Ne, Op::Lt, Op::Gt, Op::Le, Op::Ge], Self::concat)
    }

    fn concat(&mut self) -> Result<Node, ParseError> {
        self.left_assoc(&[Op::Concat], Self::additive)
    }

    fn additive(&mut self) -> Result<Node, ParseError> {
        self.left_assoc(&[Op::Add, Op::Sub], Self::multiplicative)
    }

    fn multiplicative(&mut self) -> Result<Node, ParseError> {
        self.left_assoc(&[Op::Mul, Op::Div], Self::power)
    }

    fn power(&mut self) -> Result<Node, ParseError> {
        self.left_assoc(&[Op::Pow], Self::unary)
    }

    /// Signs, then `%` after the operand. Signs are counted, not recursed into.
    fn unary(&mut self) -> Result<Node, ParseError> {
        let mut negative = false;
        let mut signs = 0usize;
        while let Some(t @ (Tok::Plus | Tok::Minus)) = self.peek() {
            negative ^= *t == Tok::Minus;
            signs += 1;
            self.pos += 1;
        }
        let (mut e, mut depth) = self.primary()?;
        while self.peek() == Some(&Tok::Percent) {
            self.pos += 1;
            e = Expr::Percent(Box::new(e));
            depth += 1;
        }
        if negative {
            e = Expr::Neg(Box::new(e));
            depth += 1;
        }
        if depth > crate::MAX_DEPTH || signs > crate::MAX_DEPTH {
            return err(self.at(), format!("formula nested more than {} deep", crate::MAX_DEPTH));
        }
        Ok((e, depth))
    }

    fn primary(&mut self) -> Result<Node, ParseError> {
        let at = self.at();
        let Some((_, tok)) = self.tokens.get(self.pos).cloned() else { return err(at, "the formula ends where a value is expected") };
        self.pos += 1;
        match tok {
            Tok::Number(n) => Ok((Expr::Number(n), 1)),
            Tok::Text(s) => Ok((Expr::Text(s), 1)),
            Tok::Field(f) => Ok((Expr::Field(f), 1)),
            Tok::Open => {
                let (e, d) = self.nested(at, Self::comparison)?;
                if self.peek() != Some(&Tok::Close) {
                    return err(self.at(), "`(` without its closing `)`");
                }
                self.pos += 1;
                Ok((e, d))
            }
            Tok::Name(name) => {
                if self.peek() != Some(&Tok::Open) {
                    return match name.to_ascii_uppercase().as_str() {
                        "TRUE" => Ok((Expr::Bool(true), 1)),
                        "FALSE" => Ok((Expr::Bool(false), 1)),
                        _ => err(at, format!("unknown name `{name}` (fields go in brackets: [{name}]; text in quotes: \"{name}\")")),
                    };
                }
                self.pos += 1;
                let func = Func::lookup(&name).ok_or_else(|| ParseError { position: at, message: format!("unknown function `{name}`") })?;
                self.call(func, at)
            }
            Tok::Close => err(at, "a value is missing before `)`"),
            Tok::Sep => err(at, "a value is missing before the separator"),
            Tok::Op(_) | Tok::Percent | Tok::Plus | Tok::Minus => err(at, "a value is missing before the operator"),
        }
    }

    /// Runs a nested parse, bounding the depth before recursing.
    fn nested(&mut self, at: usize, f: fn(&mut Self) -> Result<Node, ParseError>) -> Result<Node, ParseError> {
        if self.level >= crate::MAX_DEPTH {
            return err(at, format!("formula nested more than {} deep", crate::MAX_DEPTH));
        }
        self.level += 1;
        let node = f(self);
        self.level -= 1;
        node
    }

    /// The arguments of a call, after its `(`.
    fn call(&mut self, func: Func, at: usize) -> Result<Node, ParseError> {
        let mut args = Vec::new();
        let mut depth = 0;
        if self.peek() == Some(&Tok::Close) {
            self.pos += 1;
        } else {
            loop {
                let (mut arg, d) = self.nested(at, Self::comparison)?;
                if func.takes_range(args.len())
                    && let Expr::Field(name) = arg
                {
                    arg = Expr::Range(name);
                }
                args.push(arg);
                depth = depth.max(d);
                if args.len() > crate::MAX_ARGS {
                    return err(at, format!("more than {} arguments", crate::MAX_ARGS));
                }
                match self.peek() {
                    Some(Tok::Sep) => self.pos += 1,
                    Some(Tok::Close) => {
                        self.pos += 1;
                        break;
                    }
                    _ => return err(self.at(), format!("`{}(` without its closing `)`", func.name())),
                }
            }
        }
        let (min, max) = func.arity();
        if args.len() < min || args.len() > max {
            let wanted = if min == max {
                format!("{min}")
            } else if max >= crate::MAX_ARGS {
                format!("at least {min}")
            } else {
                format!("{min} to {max}")
            };
            return err(at, format!("{} takes {wanted} argument{}, not {}", func.name(), if max == 1 { "" } else { "s" }, args.len()));
        }
        if func == Func::Ifs && args.len() % 2 != 0 {
            return err(at, "IFS takes conditions and values in pairs: IFS(test1, value1, test2, value2…)");
        }
        if func == Func::Param && !matches!(args.first(), Some(Expr::Text(_))) {
            return err(at, "PARAM takes a parameter name in quotes: PARAM(\"period\")");
        }
        let row_independent = !args.iter().any(Expr::uses_row);
        let depth = depth + 1;
        if depth > crate::MAX_DEPTH {
            return err(at, format!("formula nested more than {} deep", crate::MAX_DEPTH));
        }
        Ok((Expr::Call { func, args, row_independent }, depth))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_follows_excel() {
        let p = |s: &str| parse(s).unwrap();
        assert_eq!(
            p("=1+2*3"),
            Expr::Binary(
                Op::Add,
                Box::new(Expr::Number(1.0)),
                Box::new(Expr::Binary(Op::Mul, Box::new(Expr::Number(2.0)), Box::new(Expr::Number(3.0))))
            )
        );
        // Negation binds tighter than `^`.
        assert_eq!(p("-2^2"), Expr::Binary(Op::Pow, Box::new(Expr::Neg(Box::new(Expr::Number(2.0)))), Box::new(Expr::Number(2.0))));
        assert!(matches!(p("[a] & \"x\" = \"1x\""), Expr::Binary(Op::Eq, ..)));
        assert_eq!(p("\"say \"\"hi\"\"\""), Expr::Text("say \"hi\"".into()));
        assert_eq!(p("[ order.date ]"), Expr::Field("order.date".into()));
        assert_eq!(p("1.5e3"), Expr::Number(1500.0));
    }

    #[test]
    fn fields_in_aggregates_are_ranges() {
        let e = parse("SUM([amount]) / COUNT([id]) + [amount] + SUM([amount]*2)").unwrap();
        assert!(e.uses_ranges() && e.uses_row());
        assert_eq!(e.fields(), ["amount", "id"]);
        let Expr::Binary(_, left, _) = parse("SUMIF([branch]; \"Lyon\"; [amount]) + 0").unwrap() else { panic!() };
        let Expr::Call { args, row_independent, .. } = *left else { panic!() };
        assert_eq!(args, [Expr::Range("branch".into()), Expr::Text("Lyon".into()), Expr::Range("amount".into())]);
        assert!(row_independent);
        assert_eq!(parse("PARAM(\"period\") & PARAM(\"x\")").unwrap().params(), ["period", "x"]);
    }

    #[test]
    fn errors_say_what_and_where() {
        let e = |s: &str| parse(s).unwrap_err();
        assert!(e("1 +").message.contains("ends"));
        assert_eq!(e("1 + foo").position, 4);
        assert!(e("FOO(1)").message.contains("unknown function `FOO`"));
        assert!(e("ROUND(1)").message.contains("takes 2 arguments, not 1"));
        assert!(e("IF(1,2,3,4)").message.contains("2 to 3"));
        assert!(e("\"abc").message.contains("closing"));
        assert!(e("[abc").message.contains("closing"));
        assert!(e("(1+2").message.contains("closing"));
        assert!(e("1+2)").message.contains("matching"));
        assert!(e("1 2").message.contains("after the formula"));
        assert!(e("PARAM([x])").message.contains("quotes"));
        assert!(e("IFS(TRUE, 1, FALSE)").message.contains("pairs"));
        assert!(e("1 # 2").message.contains('#'));
        assert!(e("").message.contains("ends"));
    }

    #[test]
    fn hostile_formulas_are_refused_not_overflowed() {
        let deep = format!("{}1{}", "(".repeat(10_000), ")".repeat(10_000));
        assert!(parse(&deep).is_err());
        let calls = format!("{}1{}", "ABS(".repeat(500), ")".repeat(500));
        assert!(parse(&calls).is_err());
        let chain = vec!["1"; 3_000].join("+");
        assert!(parse(&chain).unwrap_err().message.contains("nested"));
        let signs = format!("{}1", "-".repeat(5_000));
        assert!(parse(&signs).is_err());
        assert!(parse(&"1".repeat(crate::MAX_LEN + 1)).is_err());
        let ok = vec!["1"; 90].join("+");
        assert!(parse(&ok).is_ok());
    }
}
