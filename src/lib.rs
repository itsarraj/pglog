//! `pglog` — parses a Postgres log written with `log_min_duration_statement`
//! enabled and reports the top slow queries by normalized fingerprint.
//! See the README for scope and honest verification status.

pub mod aggregate;
pub mod fingerprint;
pub mod parser;
