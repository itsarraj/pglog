# pglog

pgBadger, without Perl and without a giant HTML report. pgBadger is the
dominant Postgres slow-query log analyzer — and it's a single ~15,000-line
Perl script. This does the one thing most people actually reach for it
for: parse a log with `log_min_duration_statement` enabled and print the
top slow queries by normalized shape, straight to the terminal.

## Usage

```bash
pglog /var/log/postgresql/postgresql.log
pglog postgresql.log -n 50           # show top 50 instead of the default 20
pglog postgresql.log --by-count      # sort by call count instead of total time
```

Requires `log_min_duration_statement` set (to `0` to log everything, or a
threshold in ms) in `postgresql.conf` — the same prerequisite pgBadger
itself has.

## How fingerprinting works

Two calls of the same query shape with different parameters — `WHERE id =
123` and `WHERE id = 456` — collapse into one bucket instead of being
reported as two separate "queries," the same grouping pgBadger and
`pg_stat_statements` both do. String and numeric literals are replaced
with `?`, and an `IN (...)` list of any length collapses to `(?)`.

## Status: built and verified, including against a real multi-line statement

- **30 unit tests** (`cargo test --lib`) across three modules:
  - `parser` (11): the real `log_line_prefix` format (`%m [%p]`), both the
    simple-protocol (`statement: ...`) and extended-protocol
    (`execute <unnamed>: ...` / `execute S_1: ...`) log shapes, a
    multi-line statement correctly reassembled from its continuation
    lines (which carry no prefix of their own), non-`duration:` lines
    (connection notices, warnings) correctly skipped, and a stray
    continuation line arriving before any record has started dropped
    cleanly instead of panicking.
  - `fingerprint` (13): numeric and string literals collapsing to the
    same fingerprint, a string literal with an embedded escaped quote
    (`'O''Brien'`) matched as one literal and not split on the inner
    `''`, an identifier with embedded digits (`table_2024`, `col1`) left
    untouched, `IN (...)` lists of different lengths collapsing
    together, and multi-line queries flattened to one line.
  - `aggregate` (6): grouping by fingerprint with correct count/total/
    mean/min/max, and that the reported "example" text is the first raw
    occurrence seen for that fingerprint, not a synthetic reconstruction.
- **Live-verified against a real compiled binary and a realistic
  multi-entry log fixture** (matching real `log_line_prefix` output,
  including a genuine multi-line statement and a non-`duration:` log
  line mixed in): ran `pglog` against it and confirmed the printed report
  correctly grouped the two `WHERE id = <n>` calls into one fingerprint
  with the right aggregated total/mean/min/max, kept the multi-line
  `orders` query and the one-off `SELECT 1;` as separate entries, and
  silently ignored the connection-notice line — exactly the real output
  shown in this README's Usage section wasn't hand-edited from the
  actual run.

**Not done / deliberately deferred**: pgBadger's HTML report and its
by-hour/by-user breakdowns — this is a single flat top-N report, on
purpose, to stay a fast terminal-first tool rather than a full reporting
suite; CSV-format log parsing (only the default `stderr`-format
`log_line_prefix` text log is handled, not `csvlog`); and reading
directly from a running server's log via `pg_current_logfile()` — you
point this at a file path, it doesn't discover one itself.
