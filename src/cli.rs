use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Clone, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Use a deterministic offline market instead of remote providers.
    #[arg(long)]
    pub demo: bool,

    /// Never make network requests; show the existing cache immediately.
    #[arg(long)]
    pub offline: bool,

    /// Override the SQLite cache location.
    #[arg(long, env = "STOCK_TUI_DB_PATH")]
    pub db: Option<PathBuf>,

    /// Alpaca market-data feed (usually iex, delayed_sip, or sip).
    #[arg(long, env = "STOCK_TUI_FEED")]
    pub feed: Option<String>,

    /// Snapshot refresh cadence in seconds.
    #[arg(long, env = "STOCK_TUI_REFRESH_SECONDS")]
    pub refresh_seconds: Option<u64>,

    /// Replace the selected cache with fresh demo data before launch.
    #[arg(long, requires = "demo")]
    pub reset_demo: bool,

    /// Print non-secret effective configuration and exit.
    #[arg(long)]
    pub print_config: bool,
}
