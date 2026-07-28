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

    /// Market-data provider adapter.
    #[arg(long, env = "STOCK_TUI_PROVIDER")]
    pub provider: Option<String>,

    /// Base URL for the provider-neutral stock-api adapter.
    #[arg(long, env = "STOCK_TUI_STOCK_API_URL")]
    pub stock_api_url: Option<String>,

    /// Whether the stock-api adapter should request its optional news endpoint.
    #[arg(long, env = "STOCK_TUI_STOCK_API_NEWS")]
    pub stock_api_news: Option<bool>,

    /// Alpaca market-data feed (usually iex, delayed_sip, or sip).
    #[arg(long, env = "STOCK_TUI_FEED")]
    pub feed: Option<String>,

    /// Remote SEC catalog URL used to refresh the embedded fallback.
    #[arg(long, env = "STOCK_TUI_CATALOG_URL")]
    pub catalog_url: Option<String>,

    /// Recheck the remote SEC catalog after this many hours.
    #[arg(long, env = "STOCK_TUI_CATALOG_REFRESH_HOURS")]
    pub catalog_refresh_hours: Option<u64>,

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

#[cfg(test)]
mod tests {
    use clap::error::ErrorKind;

    use super::*;

    #[test]
    fn short_and_long_help_are_supported_and_list_options() {
        for flag in ["-h", "--help"] {
            let error =
                Cli::try_parse_from(["stock-tui", flag]).expect_err("help exits before startup");
            assert_eq!(error.kind(), ErrorKind::DisplayHelp);

            let help = error.to_string();
            assert!(help.contains("Usage: stock-tui"));
            assert!(help.contains("--demo"));
            assert!(help.contains("--offline"));
            assert!(help.contains("--catalog-url"));
            assert!(help.contains("--stock-api-url"));
        }
    }

    #[test]
    fn stock_api_selection_and_endpoint_are_explicit() {
        let cli = Cli::try_parse_from([
            "stock-tui",
            "--provider",
            "stock-api",
            "--stock-api-url",
            "http://127.0.0.1:8787",
            "--stock-api-news",
            "false",
        ])
        .expect("stock-api options");

        assert_eq!(cli.provider.as_deref(), Some("stock-api"));
        assert_eq!(cli.stock_api_url.as_deref(), Some("http://127.0.0.1:8787"));
        assert_eq!(cli.stock_api_news, Some(false));
    }
}
