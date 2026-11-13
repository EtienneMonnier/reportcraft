//! The query log: every database query the session ran, newest last. It records what ran, where,
//! when, how long it took and how many rows came back, or why it failed. Connection strings are
//! never kept (only `user@host:port/database`); bound values are kept, as they come from the
//! report's parameters, which never hold secrets.

use std::collections::VecDeque;

use serde::Serialize;

/// Entries kept; the oldest go first.
pub const MAX_ENTRIES: usize = 200;

/// Characters of SQL kept per entry.
const MAX_SQL: usize = 4_000;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryLogEntry {
    /// When the query started, RFC 3339 in UTC.
    pub at: String,
    /// The command that ran it (`data.loadSqlite`).
    pub command: String,
    /// `sqlite`, `postgres`, `sqlserver`.
    pub source: String,
    /// The database file, or `user@host:port/database`.
    pub target: String,
    /// The SQL as sent, with placeholders (`?1`, `$1`, `@P1`).
    pub sql: String,
    /// The placeholders' values, in order (`null` for NULL).
    pub values: Vec<Option<String>>,
    pub duration_ms: u64,
    /// Rows read; absent when the query failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Served from the query cache: the database wasn't queried.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub from_cache: bool,
}

#[derive(Debug, Clone, Default)]
pub struct QueryLog {
    entries: VecDeque<QueryLogEntry>,
    /// Entries dropped to stay within `MAX_ENTRIES`.
    dropped: usize,
}

impl QueryLog {
    pub fn push(&mut self, mut entry: QueryLogEntry) {
        if let Some((cut, _)) = entry.sql.char_indices().nth(MAX_SQL) {
            entry.sql.truncate(cut);
            entry.sql.push('…');
        }
        match &entry.error {
            Some(e) => log::warn!("{} on {} failed after {} ms: {e}", entry.source, entry.target, entry.duration_ms),
            None if entry.from_cache => log::info!("{} on {}: {} rows from the cache", entry.source, entry.target, entry.rows.unwrap_or(0)),
            None => log::info!("{} on {}: {} rows in {} ms", entry.source, entry.target, entry.rows.unwrap_or(0), entry.duration_ms),
        }
        if self.entries.len() == MAX_ENTRIES {
            self.entries.pop_front();
            self.dropped += 1;
        }
        self.entries.push_back(entry);
    }

    /// The last `n` entries, oldest first.
    pub fn last(&self, n: usize) -> impl Iterator<Item = &QueryLogEntry> {
        self.entries.iter().skip(self.entries.len().saturating_sub(n))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn dropped(&self) -> usize {
        self.dropped
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.dropped = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: usize) -> QueryLogEntry {
        QueryLogEntry {
            at: String::new(),
            command: "c".into(),
            source: "sqlite".into(),
            target: "t".into(),
            sql: format!("SELECT {n}"),
            values: Vec::new(),
            duration_ms: 0,
            rows: Some(n),
            error: None,
            from_cache: false,
        }
    }

    #[test]
    fn keeps_the_newest_entries() {
        let mut log = QueryLog::default();
        for n in 0..MAX_ENTRIES + 5 {
            log.push(entry(n));
        }
        assert_eq!((log.len(), log.dropped()), (MAX_ENTRIES, 5));
        let last: Vec<_> = log.last(2).map(|e| e.rows).collect();
        assert_eq!(last, [Some(MAX_ENTRIES + 3), Some(MAX_ENTRIES + 4)]);
        assert_eq!(log.last(10_000).count(), MAX_ENTRIES);
        log.clear();
        assert!(log.is_empty() && log.dropped() == 0);
    }

    #[test]
    fn long_sql_is_shortened() {
        let mut log = QueryLog::default();
        log.push(QueryLogEntry { sql: "é".repeat(MAX_SQL + 10), ..entry(0) });
        let sql = &log.last(1).next().unwrap().sql;
        assert_eq!(sql.chars().count(), MAX_SQL + 1);
        assert!(sql.ends_with('…'));
    }
}
