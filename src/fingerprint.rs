//! Normalizing a raw SQL statement into a fingerprint so that
//! `WHERE id = 123` and `WHERE id = 456` — the same query shape run with
//! different parameters — collapse into one bucket instead of two,
//! matching how `pg_stat_statements`/pgBadger group queries for a "top
//! slow queries" report.

use regex::Regex;
use std::sync::OnceLock;

fn string_literal_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // A single-quoted literal, with SQL's own escape for an embedded quote
    // (`''`) handled by letting the alternation consume it a pair at a
    // time rather than ending the match early.
    RE.get_or_init(|| Regex::new(r"'(?:[^']|'')*'").unwrap())
}

fn numeric_literal_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // `\b` on both sides so `table_2024` or `col1` are left alone — there
    // is no word-boundary between a letter/underscore and a following
    // digit, so this only ever matches a standalone number.
    RE.get_or_init(|| Regex::new(r"\b\d+(?:\.\d+)?\b").unwrap())
}

fn in_list_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // After literals are gone, `IN (?, ?, ?)` and `IN (?, ?)` are the same
    // query shape with a different-length parameter list — collapse any
    // run of two or more placeholders in parens down to one.
    RE.get_or_init(|| Regex::new(r"\(\s*\?(?:\s*,\s*\?)+\s*\)").unwrap())
}

fn whitespace_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\s+").unwrap())
}

/// Flatten a possibly multi-line raw query into one normalized-whitespace
/// line, with no fingerprinting applied. Used both as the fingerprint's
/// first step and to build the human-readable "example" text a report
/// shows alongside each fingerprint.
pub fn flatten(query: &str) -> String {
    whitespace_re().replace_all(query.trim(), " ").into_owned()
}

/// Collapse a query down to its shape: literals and numbers become `?`,
/// `IN` lists of any length become `(?)`, and whitespace is normalized —
/// so two calls with different parameters produce the same fingerprint.
pub fn normalize(query: &str) -> String {
    let flat = flatten(query);
    let no_strings = string_literal_re().replace_all(&flat, "?");
    let no_numbers = numeric_literal_re().replace_all(&no_strings, "?");
    let no_lists = in_list_re().replace_all(&no_numbers, "(?)");
    whitespace_re()
        .replace_all(no_lists.trim(), " ")
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn different_numeric_literals_collapse_to_the_same_fingerprint() {
        let a = normalize("SELECT * FROM users WHERE id = 123");
        let b = normalize("SELECT * FROM users WHERE id = 456");
        assert_eq!(a, b);
        assert_eq!(a, "SELECT * FROM users WHERE id = ?");
    }

    #[test]
    fn different_string_literals_collapse_to_the_same_fingerprint() {
        let a = normalize("SELECT * FROM users WHERE name = 'alice'");
        let b = normalize("SELECT * FROM users WHERE name = 'bob'");
        assert_eq!(a, b);
    }

    #[test]
    fn string_literal_with_embedded_escaped_quote_is_matched_whole() {
        // `'O''Brien'` is one SQL string literal, not two literals with an
        // unmatched quote in between.
        let out = normalize("SELECT * FROM users WHERE name = 'O''Brien'");
        assert_eq!(out, "SELECT * FROM users WHERE name = ?");
    }

    #[test]
    fn identifiers_with_embedded_digits_are_left_alone() {
        let out = normalize("SELECT * FROM table_2024 WHERE col1 = 5");
        assert_eq!(out, "SELECT * FROM table_2024 WHERE col1 = ?");
    }

    #[test]
    fn decimal_numbers_collapse_as_one_token() {
        let out = normalize("SELECT * FROM t WHERE price > 19.99");
        assert_eq!(out, "SELECT * FROM t WHERE price > ?");
    }

    #[test]
    fn in_lists_of_different_lengths_collapse_to_the_same_fingerprint() {
        let a = normalize("SELECT * FROM t WHERE id IN (1, 2, 3)");
        let b = normalize("SELECT * FROM t WHERE id IN (1, 2, 3, 4, 5)");
        assert_eq!(a, b);
        assert_eq!(a, "SELECT * FROM t WHERE id IN (?)");
    }

    #[test]
    fn single_element_in_list_is_still_just_one_placeholder() {
        let out = normalize("SELECT * FROM t WHERE id IN (1)");
        assert_eq!(out, "SELECT * FROM t WHERE id IN (?)");
    }

    #[test]
    fn multiline_query_is_flattened_to_one_line() {
        let out = normalize("SELECT *\nFROM foo\nWHERE id = 1;");
        assert_eq!(out, "SELECT * FROM foo WHERE id = ?;");
    }

    #[test]
    fn extra_internal_whitespace_is_collapsed() {
        let a = normalize("SELECT   *  FROM   t   WHERE id = 1");
        let b = normalize("SELECT * FROM t WHERE id = 2");
        assert_eq!(a, b);
    }

    #[test]
    fn leading_and_trailing_whitespace_is_trimmed() {
        assert_eq!(normalize("  SELECT 1  "), "SELECT ?");
    }

    #[test]
    fn queries_with_genuinely_different_shape_do_not_collapse() {
        let a = normalize("SELECT * FROM users WHERE id = 1");
        let b = normalize("SELECT * FROM orders WHERE id = 1");
        assert_ne!(a, b);
    }

    #[test]
    fn flatten_does_not_touch_literal_values() {
        assert_eq!(
            flatten("SELECT * FROM t\nWHERE id = 123"),
            "SELECT * FROM t WHERE id = 123"
        );
    }

    #[test]
    fn empty_string_normalizes_to_empty() {
        assert_eq!(normalize(""), "");
    }
}
