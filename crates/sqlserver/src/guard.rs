//! The first line of defence for report queries: a statement must read, and only read. SQL Server
//! has no read-only transaction, so the query text is scanned (comments, strings and quoted names
//! skipped) and must be one statement starting with `SELECT` or `WITH`, with none of the words
//! that write, run code, change the session or end the transaction. The query then runs inside a
//! transaction that is never committed (the second line), and a login that can only read (role
//! `db_datareader`) is the guarantee.

use reportcraft_core::{Error, Result};

/// Words a query that only reads never needs. `SELECT … INTO` creates a table, `COMMIT` would
/// end the transaction that is rolled back, `SET` and `USE` change the session, `EXEC` runs
/// arbitrary code, `OPENROWSET` and friends reach other servers.
const FORBIDDEN: &[&str] = &[
    "ALTER",
    "BACKUP",
    "BEGIN",
    "BULK",
    "CHECKPOINT",
    "COMMIT",
    "CREATE",
    "DBCC",
    "DECLARE",
    "DELETE",
    "DENY",
    "DROP",
    "EXEC",
    "EXECUTE",
    "GRANT",
    "INSERT",
    "INTO",
    "KILL",
    "MERGE",
    "OPENDATASOURCE",
    "OPENQUERY",
    "OPENROWSET",
    "READTEXT",
    "RECONFIGURE",
    "RESTORE",
    "REVOKE",
    "ROLLBACK",
    "SAVE",
    "SET",
    "SHUTDOWN",
    "TRUNCATE",
    "UPDATE",
    "UPDATETEXT",
    "USE",
    "WAITFOR",
    "WRITETEXT",
];

/// What the scan found.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Scan {
    /// The words outside comments, strings and quoted names, upper-cased (`@` variables left out).
    words: Vec<String>,
    /// The highest `@Pn` placeholder.
    pub placeholders: usize,
    /// A `;` followed by more than blanks and comments.
    several: bool,
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '@' | '#' | '$')
}

/// Skips a quoted run that starts at `chars[i]` (`'…'`, `"…"` or `[…]`, the closing mark doubled
/// inside); returns the index after it, or the end when it never closes.
fn skip_quoted(chars: &[char], i: usize, close: char) -> usize {
    let mut j = i + 1;
    while j < chars.len() {
        if chars[j] == close {
            if chars.get(j + 1) == Some(&close) {
                j += 2;
                continue;
            }
            return j + 1;
        }
        j += 1;
    }
    j
}

/// Skips a `/* … */` comment (they nest in T-SQL) starting at `chars[i]`.
fn skip_block(chars: &[char], i: usize) -> usize {
    let mut depth = 0usize;
    let mut j = i;
    while j < chars.len() {
        if chars[j] == '/' && chars.get(j + 1) == Some(&'*') {
            depth += 1;
            j += 2;
        } else if chars[j] == '*' && chars.get(j + 1) == Some(&'/') {
            depth = depth.saturating_sub(1);
            j += 2;
            if depth == 0 {
                return j;
            }
        } else {
            j += 1;
        }
    }
    j
}

pub(crate) fn scan(sql: &str) -> Scan {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Scan::default();
    let mut after_semicolon = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '-' && next == Some('-') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && next == Some('*') {
            i = skip_block(&chars, i);
            continue;
        }
        if after_semicolon {
            out.several = true;
        }
        match c {
            ';' => {
                after_semicolon = true;
                i += 1;
            }
            '\'' => i = skip_quoted(&chars, i, '\''),
            '"' => i = skip_quoted(&chars, i, '"'),
            '[' => i = skip_quoted(&chars, i, ']'),
            c if c.is_ascii_digit() => {
                // A number (`12`, `1.5e3`, `0x1F`): never a word.
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '.') {
                    i += 1;
                }
            }
            c if word_char(c) => {
                let start = i;
                while i < chars.len() && word_char(chars[i]) {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect::<String>().to_uppercase();
                if let Some(rest) = word.strip_prefix("@P")
                    && let Ok(n) = rest.parse::<usize>()
                {
                    out.placeholders = out.placeholders.max(n);
                } else if !word.starts_with('@') {
                    out.words.push(word);
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// Checks that `sql` is one statement that only reads; returns its highest `@Pn` placeholder.
pub(crate) fn check(sql: &str) -> Result<usize> {
    let scan = scan(sql);
    let Some(first) = scan.words.first() else { return Err(Error::Invalid("the query is empty".into())) };
    if scan.several {
        return Err(Error::Invalid("one statement only: remove what follows the `;`".into()));
    }
    if first != "SELECT" && first != "WITH" {
        return Err(Error::Invalid("only queries that read data are allowed (SELECT, WITH …)".into()));
    }
    if let Some(word) = scan.words.iter().find(|w| FORBIDDEN.contains(&w.as_str())) {
        return Err(Error::Invalid(format!(
            "only queries that read data are allowed: `{word}` is not (a column with that name goes in brackets, [{word}])"
        )));
    }
    Ok(scan.placeholders)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_queries_pass() {
        assert_eq!(check("SELECT * FROM rental").ok(), Some(0));
        assert_eq!(check("  with t AS (SELECT 1 AS a) SELECT a FROM t;  -- done").ok(), Some(0));
        assert_eq!(check("SELECT [update], 'DELETE ; DROP' AS x FROM [into] /* SET */ WHERE a = @P2 OR b = @p1").ok(), Some(2));
        assert_eq!(check("SELECT \"set\" FROM t WHERE n = N'it''s; INSERT'").ok(), Some(0));
        assert_eq!(check("SELECT 0x1F, 1.5e3, CASE WHEN a > 1 THEN 'x' END FROM t WITH (NOLOCK)").ok(), Some(0));
        assert_eq!(check("/* a /* nested */ SET */ SELECT 1").ok(), Some(0));
    }

    #[test]
    fn writing_or_several_statements_fail() {
        for sql in [
            "DELETE FROM rental",
            "SELECT * INTO copy FROM rental",
            "SELECT 1 DELETE FROM rental",
            "SELECT 1; DELETE FROM rental",
            "SELECT 1; SELECT 2",
            "WITH t AS (SELECT 1 AS a) DELETE FROM t",
            "SELECT 1 COMMIT",
            "EXEC sp_who",
            "SELECT * FROM OPENROWSET('SQLNCLI', 'x', 'SELECT 1')",
            "  ",
            "-- only a comment",
        ] {
            assert!(check(sql).is_err(), "{sql}");
        }
        let e = check("SELECT 1 SET NOCOUNT ON").unwrap_err().to_string();
        assert!(e.contains("`SET`") && e.contains("[SET]"), "{e}");
    }
}
