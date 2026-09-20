//! Grouping parsed log entries by fingerprint and rolling up the numbers
//! a "top slow queries" report actually needs.

use std::collections::BTreeMap;

use crate::fingerprint;
use crate::parser::LogEntry;

/// One fingerprint's rolled-up stats across every occurrence in the log.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryStats {
    pub fingerprint: String,
    /// The first raw query text seen for this fingerprint, flattened to
    /// one line — shown in a report as a concrete example of the shape.
    pub example: String,
    pub count: u64,
    pub total_ms: f64,
    pub mean_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
}

/// Group `entries` by normalized fingerprint. Order of the returned
/// `Vec` is by fingerprint text (stable and deterministic) — callers sort
/// it into whatever presentation order they actually want.
pub fn aggregate(entries: &[LogEntry]) -> Vec<QueryStats> {
    struct Acc {
        example: String,
        count: u64,
        total_ms: f64,
        min_ms: f64,
        max_ms: f64,
    }

    let mut by_fp: BTreeMap<String, Acc> = BTreeMap::new();
    for entry in entries {
        let fp = fingerprint::normalize(&entry.query);
        let acc = by_fp.entry(fp).or_insert_with(|| Acc {
            example: fingerprint::flatten(&entry.query),
            count: 0,
            total_ms: 0.0,
            min_ms: f64::MAX,
            max_ms: f64::MIN,
        });
        acc.count += 1;
        acc.total_ms += entry.duration_ms;
        acc.min_ms = acc.min_ms.min(entry.duration_ms);
        acc.max_ms = acc.max_ms.max(entry.duration_ms);
    }

    by_fp
        .into_iter()
        .map(|(fingerprint, acc)| QueryStats {
            fingerprint,
            example: acc.example,
            count: acc.count,
            total_ms: acc.total_ms,
            mean_ms: acc.total_ms / acc.count as f64,
            min_ms: acc.min_ms,
            max_ms: acc.max_ms,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(query: &str, ms: f64) -> LogEntry {
        LogEntry {
            pid: 1,
            duration_ms: ms,
            query: query.to_string(),
        }
    }

    #[test]
    fn groups_different_parameters_into_one_bucket() {
        let entries = vec![
            entry("SELECT * FROM users WHERE id = 1", 10.0),
            entry("SELECT * FROM users WHERE id = 2", 20.0),
        ];
        let stats = aggregate(&entries);
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].count, 2);
        assert_eq!(stats[0].total_ms, 30.0);
        assert_eq!(stats[0].mean_ms, 15.0);
        assert_eq!(stats[0].min_ms, 10.0);
        assert_eq!(stats[0].max_ms, 20.0);
    }

    #[test]
    fn keeps_genuinely_different_queries_separate() {
        let entries = vec![
            entry("SELECT * FROM users WHERE id = 1", 10.0),
            entry("SELECT * FROM orders WHERE id = 1", 10.0),
        ];
        assert_eq!(aggregate(&entries).len(), 2);
    }

    #[test]
    fn example_is_the_first_raw_occurrence_flattened() {
        let entries = vec![
            entry("SELECT * FROM users WHERE id = 42", 10.0),
            entry("SELECT * FROM users WHERE id = 99", 10.0),
        ];
        let stats = aggregate(&entries);
        assert_eq!(stats[0].example, "SELECT * FROM users WHERE id = 42");
    }

    #[test]
    fn empty_entries_produce_no_stats() {
        assert!(aggregate(&[]).is_empty());
    }

    #[test]
    fn single_entry_min_and_max_equal_its_own_duration() {
        let entries = vec![entry("SELECT 1", 5.0)];
        let stats = aggregate(&entries);
        assert_eq!(stats[0].min_ms, 5.0);
        assert_eq!(stats[0].max_ms, 5.0);
        assert_eq!(stats[0].mean_ms, 5.0);
    }

    #[test]
    fn three_distinct_fingerprints_produce_three_buckets() {
        let entries = vec![
            entry("SELECT 1", 1.0),
            entry("SELECT 2", 1.0),
            entry("UPDATE t SET x = 1", 1.0),
            entry("UPDATE t SET x = 2", 1.0),
            entry("DELETE FROM t", 1.0),
        ];
        // "SELECT 1"/"SELECT 2" -> one bucket, "UPDATE ..." -> one bucket,
        // "DELETE FROM t" -> one bucket.
        assert_eq!(aggregate(&entries).len(), 3);
    }
}
