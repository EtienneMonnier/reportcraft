//! The query cache: results kept for a while so a report can be rendered again (other
//! parameters for the layout, another format) without querying the database again. A load uses
//! it only when it asks to (`cacheSeconds`), and says when the rows it hands over were read.
// The web build has no clock and no database to cache: the store is unused there.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use std::collections::VecDeque;
use std::sync::Arc;

use reportcraft_core::RowSet;

/// Results kept at most.
const MAX_ENTRIES: usize = 8;
/// Memory the kept results may take at most (an estimate, see `Value::size`).
const MAX_BYTES: usize = 1 << 30;

#[derive(Debug, Clone)]
struct Entry {
    key: String,
    rows: Arc<RowSet>,
    /// When the rows were read, RFC 3339 in local time.
    refreshed_at: String,
    stored: std::time::Instant,
    bytes: usize,
}

#[derive(Debug, Clone, Default)]
pub struct QueryCache {
    entries: VecDeque<Entry>,
}

/// A cached result: the rows and when they were read.
#[derive(Debug, Clone)]
pub struct Hit {
    pub rows: Arc<RowSet>,
    pub refreshed_at: String,
}

impl QueryCache {
    /// The rows stored under `key`, if read less than `max_age` ago.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn get(&self, key: &str, max_age: std::time::Duration) -> Option<Hit> {
        let e = self.entries.iter().find(|e| e.key == key)?;
        (e.stored.elapsed() <= max_age).then(|| Hit { rows: Arc::clone(&e.rows), refreshed_at: e.refreshed_at.clone() })
    }

    /// Stores rows read now, replacing any under the same key; the oldest go first when full.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn put(&mut self, key: String, rows: Arc<RowSet>, refreshed_at: String) {
        self.entries.retain(|e| e.key != key);
        let bytes = rows.rows.iter().map(|r| r.iter().map(reportcraft_core::Value::size).sum::<usize>()).sum();
        if bytes > MAX_BYTES {
            return;
        }
        // Native only: the web build has no clock (and no database to cache).
        #[allow(clippy::disallowed_methods)]
        let stored = std::time::Instant::now();
        self.entries.push_back(Entry { key, rows, refreshed_at, stored, bytes });
        while self.entries.len() > MAX_ENTRIES || self.entries.iter().map(|e| e.bytes).sum::<usize>() > MAX_BYTES {
            self.entries.pop_front();
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn keeps_recent_results_by_key() {
        let mut c = QueryCache::default();
        c.put("a".into(), Arc::new(RowSet::default()), "t1".into());
        assert_eq!(c.get("a", Duration::from_secs(60)).map(|h| h.refreshed_at), Some("t1".into()));
        assert!(c.get("b", Duration::from_secs(60)).is_none());
        c.put("a".into(), Arc::new(RowSet::default()), "t2".into());
        assert_eq!(c.len(), 1, "same key replaced");
        for i in 0..MAX_ENTRIES + 3 {
            c.put(format!("k{i}"), Arc::new(RowSet::default()), String::new());
        }
        assert_eq!(c.len(), MAX_ENTRIES);
        assert!(c.get("a", Duration::from_secs(60)).is_none(), "the oldest went first");
        c.clear();
        assert!(c.is_empty());
    }
}
