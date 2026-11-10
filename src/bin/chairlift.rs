//! `chairlift` — command-line formatter for ClickHouse SQL.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::LazyLock;

use clap::Parser;

/// Format ClickHouse SQL and Jinja-templated SQL files.
#[derive(Debug, Parser)]
#[command(version = VERSION.as_str())]
struct Cli {
    /// Files or directories to format; `-` reads stdin and writes stdout.
    paths: Vec<PathBuf>,
}

/// `<version> (ClickHouse <version>)`, so a bug report names both.
static VERSION: LazyLock<String> = LazyLock::new(|| {
    format!(
        "{} (ClickHouse {})",
        env!("CARGO_PKG_VERSION"),
        chairlift::CLICKHOUSE_VERSION
    )
});

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.paths.is_empty() {
        return ExitCode::SUCCESS;
    }
    eprintln!("chairlift: formatting is not implemented yet");
    ExitCode::from(2)
}
