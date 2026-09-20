//! Parsing a Postgres log written with the default `log_line_prefix`
//! (`'%m [%p] '`) and `log_min_duration_statement` enabled, into the
//! individual `duration: ... statement: ...` entries it contains.
//!
//! Every log *message* starts with a prefixed line (`%m` timestamp, then
//! `[pid]`) but a message's body can still span several physical lines —
//! a multi-line SQL statement is logged with its real line breaks intact.
//! Those continuation lines carry no prefix of their own, so the first
//! job here is re-assembling "physical lines" back into "log messages"
//! before anything about `duration:`/`statement:` is even looked at.

use regex::Regex;
use std::sync::OnceLock;

/// One assembled log message: everything from one `%m [%p]`-prefixed line
/// up to (but not including) the next one.
#[derive(Debug, Clone, PartialEq)]
struct RawRecord {
    pid: i32,
    /// The message body, with embedded `\n` for any continuation lines.
    message: String,
}

fn prefix_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // `2026-09-20 00:51:18.491 IST [138615] LOG:  duration: ...`
        // Deliberately specific (a full date, then a time, then one more
        // token, then a bracketed pid) so a continuation line of raw SQL
        // text has essentially no chance of being mistaken for a new
        // record's prefix.
        Regex::new(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:\.\d+)? \S+ \[(\d+)\] (.*)$").unwrap()
    })
}

fn duration_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Covers both the simple-protocol form (`statement: ...`) and the
        // extended-protocol form (`execute <name>: ...` — `<unnamed>` for
        // an unnamed prepared statement, or the name psycopg2/pgbouncer
        // etc. assigned). `(?s)` lets `.` cross the embedded newlines a
        // multi-line statement leaves in the joined message.
        Regex::new(r"(?s)^LOG:\s+duration:\s+([0-9.]+)\s+ms\s+(?:statement|execute [^:]*):\s?(.*)$")
            .unwrap()
    })
}

fn split_records(text: &str) -> Vec<RawRecord> {
    let prefix = prefix_re();
    let mut records: Vec<RawRecord> = Vec::new();

    for line in text.lines() {
        if let Some(caps) = prefix.captures(line) {
            let pid: i32 = caps[1].parse().unwrap_or(0);
            let rest = caps[2].to_string();
            records.push(RawRecord { pid, message: rest });
        } else if let Some(last) = records.last_mut() {
            // A continuation line of the most recently started record —
            // e.g. line 2+ of a multi-line SQL statement.
            last.message.push('\n');
            last.message.push_str(line);
        }
        // A continuation line arriving before any record has started
        // (a log file that begins mid-message, or genuine garbage) is
        // silently dropped rather than crashing the whole parse.
    }

    records
}

/// One `duration:` log entry, with everything downstream (fingerprinting,
/// aggregation) needs.
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub pid: i32,
    pub duration_ms: f64,
    /// The raw query text exactly as logged — may still contain the
    /// original embedded newlines of a multi-line statement.
    pub query: String,
}

/// Parse an entire log file's text into every `duration:` entry it
/// contains. Every other log line (plain `LOG:`, `WARNING:`, `ERROR:`,
/// `STATEMENT:`, `DETAIL:`, connection/disconnection notices, ...) is
/// silently skipped — this tool only ever reports on slow-query timing.
pub fn parse(text: &str) -> Vec<LogEntry> {
    let duration = duration_re();
    split_records(text)
        .into_iter()
        .filter_map(|record| {
            let caps = duration.captures(&record.message)?;
            let duration_ms: f64 = caps[1].parse().ok()?;
            let query = caps[2].to_string();
            Some(LogEntry {
                pid: record.pid,
                duration_ms,
                query,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_single_real_shaped_duration_line() {
        let log = "2026-09-20 00:51:18.491 IST [138615] LOG:  duration: 2.968 ms  statement: SELECT * FROM pg_class WHERE oid = 123;\n";
        let entries = parse(log);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].pid, 138615);
        assert_eq!(entries[0].duration_ms, 2.968);
        assert_eq!(entries[0].query, "SELECT * FROM pg_class WHERE oid = 123;");
    }

    #[test]
    fn parses_multiple_entries_from_the_same_backend() {
        let log = "\
2026-09-20 00:51:18.488 IST [138615] LOG:  duration: 0.101 ms  statement: SET log_min_duration_statement = 0;
2026-09-20 00:51:18.491 IST [138615] LOG:  duration: 2.968 ms  statement: SELECT * FROM pg_class WHERE oid = 123;
2026-09-20 00:51:18.491 IST [138615] LOG:  duration: 0.057 ms  statement: SELECT * FROM pg_class WHERE oid = 456;
";
        let entries = parse(log);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[1].query, "SELECT * FROM pg_class WHERE oid = 123;");
        assert_eq!(entries[2].query, "SELECT * FROM pg_class WHERE oid = 456;");
    }

    #[test]
    fn skips_non_duration_log_lines() {
        let log = "\
2026-09-20 00:51:18.491 IST [138615] LOG:  connection authorized: user=app database=appdb
2026-09-20 00:51:18.492 IST [138615] WARNING:  there is already a transaction in progress
2026-09-20 00:51:18.500 IST [138615] LOG:  duration: 5.000 ms  statement: SELECT 1;
";
        let entries = parse(log);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].query, "SELECT 1;");
    }

    #[test]
    fn reassembles_a_multiline_statement() {
        let log = "\
2026-09-20 00:51:18.491 IST [138615] LOG:  duration: 5.000 ms  statement: SELECT *
FROM foo
WHERE id = 1;
2026-09-20 00:51:19.000 IST [138615] LOG:  duration: 1.000 ms  statement: SELECT 2;
";
        let entries = parse(log);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].query, "SELECT *\nFROM foo\nWHERE id = 1;");
        assert_eq!(entries[1].query, "SELECT 2;");
    }

    #[test]
    fn parses_extended_protocol_execute_form() {
        let log = "2026-09-20 00:51:18.491 IST [138615] LOG:  duration: 1.234 ms  execute <unnamed>: SELECT $1 AS x;\n";
        let entries = parse(log);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].query, "SELECT $1 AS x;");
    }

    #[test]
    fn parses_named_prepared_statement_execute_form() {
        let log = "2026-09-20 00:51:18.491 IST [138615] LOG:  duration: 1.234 ms  execute S_1: SELECT $1;\n";
        let entries = parse(log);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].query, "SELECT $1;");
    }

    #[test]
    fn empty_input_produces_no_entries() {
        assert!(parse("").is_empty());
    }

    #[test]
    fn a_log_with_no_duration_statements_produces_no_entries() {
        let log = "2026-09-20 00:51:18.491 IST [138615] LOG:  checkpoint starting: time\n";
        assert!(parse(log).is_empty());
    }

    #[test]
    fn continuation_line_before_any_record_is_dropped_not_a_panic() {
        let log = "orphan continuation line with no prefix at all\n2026-09-20 00:51:18.491 IST [1] LOG:  duration: 1.0 ms  statement: SELECT 1;\n";
        let entries = parse(log);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn pid_is_parsed_as_a_real_integer() {
        let log = "2026-09-20 00:51:18.491 IST [99] LOG:  duration: 1.0 ms  statement: SELECT 1;\n";
        assert_eq!(parse(log)[0].pid, 99);
    }

    #[test]
    fn timestamp_without_fractional_seconds_still_matches() {
        // log_line_prefix's `%m` always includes milliseconds in
        // practice, but the prefix regex shouldn't be needlessly strict.
        let log = "2026-09-20 00:51:18 IST [1] LOG:  duration: 1.0 ms  statement: SELECT 1;\n";
        assert_eq!(parse(log).len(), 1);
    }
}
