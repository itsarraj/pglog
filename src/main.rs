use std::fs;
use std::path::PathBuf;

use clap::Parser;
use pglog::{aggregate, parser};

#[derive(Parser)]
#[command(
    name = "pglog",
    about = "Top slow queries from a Postgres log with log_min_duration_statement enabled — a pgBadger alternative with no Perl and no HTML report"
)]
struct Cli {
    /// Postgres log file to parse.
    log_file: PathBuf,
    /// How many of the slowest (by total time) query fingerprints to show.
    #[arg(short = 'n', long, default_value_t = 20)]
    top: usize,
    /// Sort by total time spent (default) instead of by call count.
    #[arg(long)]
    by_count: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let content = fs::read_to_string(&cli.log_file)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", cli.log_file.display()))?;

    let entries = parser::parse(&content);
    if entries.is_empty() {
        println!(
            "no duration-logged statements found in {}",
            cli.log_file.display()
        );
        return Ok(());
    }

    let mut stats = aggregate::aggregate(&entries);
    if cli.by_count {
        stats.sort_by_key(|s| std::cmp::Reverse(s.count));
    } else {
        stats.sort_by(|a, b| b.total_ms.partial_cmp(&a.total_ms).unwrap());
    }

    let total_statements: u64 = stats.iter().map(|s| s.count).sum();
    println!(
        "{} statement(s), {} distinct fingerprint(s) — top {} by {}:\n",
        total_statements,
        stats.len(),
        cli.top.min(stats.len()),
        if cli.by_count {
            "call count"
        } else {
            "total time"
        }
    );

    for (i, s) in stats.iter().take(cli.top).enumerate() {
        println!(
            "{:>3}. {:>8} calls  total {:>10.1} ms  mean {:>8.2} ms  min {:>7.2} ms  max {:>9.2} ms",
            i + 1,
            s.count,
            s.total_ms,
            s.mean_ms,
            s.min_ms,
            s.max_ms
        );
        let example = if s.example.len() > 100 {
            format!("{}...", &s.example[..100])
        } else {
            s.example.clone()
        };
        println!("     {example}\n");
    }

    Ok(())
}
