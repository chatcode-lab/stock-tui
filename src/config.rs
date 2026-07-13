use std::{
    env, fmt, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use directories::ProjectDirs;
use secrecy::SecretString;
use serde::Deserialize;

use crate::cli::Cli;

const DEFAULT_DATA_URL: &str = "https://data.alpaca.markets";
const DEFAULT_TRADING_URL: &str = "https://paper-api.alpaca.markets";

#[derive(Clone)]
pub struct Credentials {
    pub key: SecretString,
    pub secret: SecretString,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("key", &"[redacted]")
            .field("secret", &"[redacted]")
            .finish()
    }
}

#[derive(Clone)]
pub struct Settings {
    pub credentials: Option<Credentials>,
    pub db_path: PathBuf,
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub data_url: String,
    pub trading_url: String,
    pub feed: String,
    pub refresh_interval: Duration,
    pub request_limit_per_minute: u32,
    pub snapshot_batch_size: usize,
    pub history_batch_size: usize,
    pub demo: bool,
    pub offline: bool,
    pub reset_demo: bool,
}

impl fmt::Debug for Settings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Settings")
            .field("credentials", &self.credentials)
            .field("db_path", &self.db_path)
            .field("config_dir", &self.config_dir)
            .field("cache_dir", &self.cache_dir)
            .field("data_url", &self.data_url)
            .field("trading_url", &self.trading_url)
            .field("feed", &self.feed)
            .field("refresh_interval", &self.refresh_interval)
            .field("request_limit_per_minute", &self.request_limit_per_minute)
            .field("snapshot_batch_size", &self.snapshot_batch_size)
            .field("history_batch_size", &self.history_batch_size)
            .field("demo", &self.demo)
            .field("offline", &self.offline)
            .field("reset_demo", &self.reset_demo)
            .finish()
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileConfig {
    feed: Option<String>,
    refresh_seconds: Option<u64>,
    request_limit_per_minute: Option<u32>,
    snapshot_batch_size: Option<usize>,
    history_batch_size: Option<usize>,
    data_url: Option<String>,
    trading_url: Option<String>,
}

impl Settings {
    pub fn load(cli: &Cli) -> Result<Self> {
        if let Err(error) = dotenvy::dotenv()
            && !error.not_found()
        {
            return Err(error).context("could not load local .env file");
        }
        let project = ProjectDirs::from("com", "chatcode-lab", "stock-tui")
            .context("could not determine user data directories")?;
        let config_dir = project.config_dir().to_path_buf();
        let cache_dir = project.cache_dir().to_path_buf();
        let data_dir = project.data_dir().to_path_buf();
        fs::create_dir_all(&config_dir).context("could not create configuration directory")?;
        fs::create_dir_all(&cache_dir).context("could not create cache directory")?;
        fs::create_dir_all(&data_dir).context("could not create application data directory")?;

        let file = read_file_config(&config_dir.join("config.toml"))?;
        let credentials = credentials_from_env()?;
        let feed = cli
            .feed
            .clone()
            .or_else(|| env::var("STOCK_TUI_FEED").ok())
            .or(file.feed)
            .unwrap_or_else(|| "iex".to_owned());
        if !matches!(feed.as_str(), "iex" | "sip" | "delayed_sip") {
            bail!("unsupported Alpaca feed {feed:?}; expected iex, delayed_sip, or sip");
        }
        let refresh_seconds = cli
            .refresh_seconds
            .or_else(|| env_u64("STOCK_TUI_REFRESH_SECONDS"))
            .or(file.refresh_seconds)
            .unwrap_or(300)
            .clamp(30, 86_400);
        let db_path = cli
            .db
            .clone()
            .unwrap_or_else(|| data_dir.join("market.sqlite3"));
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).context("could not create database directory")?;
        }
        let demo = resolve_demo_mode(cli.demo, cli.offline, credentials.is_some());

        Ok(Self {
            credentials,
            db_path,
            config_dir,
            cache_dir,
            data_url: env::var("STOCK_TUI_DATA_URL")
                .ok()
                .or(file.data_url)
                .unwrap_or_else(|| DEFAULT_DATA_URL.to_owned()),
            trading_url: env::var("STOCK_TUI_TRADING_URL")
                .ok()
                .or(file.trading_url)
                .unwrap_or_else(|| DEFAULT_TRADING_URL.to_owned()),
            feed,
            refresh_interval: Duration::from_secs(refresh_seconds),
            request_limit_per_minute: file.request_limit_per_minute.unwrap_or(180).clamp(1, 200),
            snapshot_batch_size: file.snapshot_batch_size.unwrap_or(100).clamp(1, 500),
            history_batch_size: file.history_batch_size.unwrap_or(50).clamp(1, 200),
            demo,
            offline: cli.offline,
            reset_demo: cli.reset_demo,
        })
    }

    #[must_use]
    pub fn mode_label(&self) -> &'static str {
        if self.demo {
            "demo"
        } else if self.offline {
            "offline cache"
        } else {
            "Alpaca"
        }
    }
}

fn read_file_config(path: &Path) -> Result<FileConfig> {
    if !path.exists() {
        return Ok(FileConfig::default());
    }
    let contents = fs::read_to_string(path)
        .with_context(|| format!("could not read configuration at {}", path.display()))?;
    toml::from_str(&contents)
        .with_context(|| format!("invalid configuration at {}", path.display()))
}

fn credentials_from_env() -> Result<Option<Credentials>> {
    let key = env::var("ALPACA_API_KEY")
        .ok()
        .filter(|value| !value.is_empty());
    let secret = env::var("ALPACA_API_SECRET")
        .ok()
        .filter(|value| !value.is_empty());
    match (key, secret) {
        (Some(key), Some(secret)) => Ok(Some(Credentials {
            key: SecretString::from(key),
            secret: SecretString::from(secret),
        })),
        (None, None) => Ok(None),
        _ => bail!("ALPACA_API_KEY and ALPACA_API_SECRET must be set together"),
    }
}

fn env_u64(key: &str) -> Option<u64> {
    env::var(key).ok()?.parse().ok()
}

fn resolve_demo_mode(requested: bool, offline: bool, has_credentials: bool) -> bool {
    requested || (!offline && !has_credentials)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_exposes_credentials() {
        let credentials = Credentials {
            key: SecretString::from("public-key".to_owned()),
            secret: SecretString::from("very-secret".to_owned()),
        };
        let rendered = format!("{credentials:?}");
        assert!(!rendered.contains("public-key"));
        assert!(!rendered.contains("very-secret"));
        assert!(rendered.contains("redacted"));
    }

    #[test]
    fn explicit_offline_mode_does_not_replace_a_live_cache_with_demo_data() {
        assert!(!resolve_demo_mode(false, true, false));
        assert!(resolve_demo_mode(true, true, false));
        assert!(resolve_demo_mode(false, false, false));
        assert!(!resolve_demo_mode(false, false, true));
    }

    #[test]
    fn default_trading_url_uses_alpaca_paper_endpoint() {
        assert_eq!(DEFAULT_TRADING_URL, "https://paper-api.alpaca.markets");
    }
}
