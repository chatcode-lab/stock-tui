use std::{
    env, fmt, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use directories::ProjectDirs;
use secrecy::SecretString;
use serde::Deserialize;

use crate::{cli::Cli, providers::stock_api::validated_bearer_token};

const DEFAULT_DATA_URL: &str = "https://data.alpaca.markets";
const DEFAULT_TRADING_URL: &str = "https://paper-api.alpaca.markets";
pub const DEFAULT_STOCK_API_URL: &str = "https://stock.chatcode.dev/api";
pub const DEFAULT_CATALOG_URL: &str = "https://stock.chatcode.dev/catalog/sec-catalog.json";
pub const CREDENTIALS_FILE_NAME: &str = "credentials.env";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Alpaca,
    StockApi,
}

impl ProviderKind {
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Alpaca => "alpaca",
            Self::StockApi => "stock-api",
        }
    }

    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Alpaca => "Alpaca",
            Self::StockApi => "Stock API",
        }
    }

    #[must_use]
    pub const fn requires_credentials(self) -> bool {
        matches!(self, Self::Alpaca)
    }

    fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "alpaca" => Ok(Self::Alpaca),
            "stock-api" | "stock_api" => Ok(Self::StockApi),
            _ => bail!("unsupported market-data provider {value:?}; expected alpaca or stock-api"),
        }
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSource {
    Environment,
    StoredFile,
    OnboardingSession,
}

impl CredentialSource {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::StoredFile => "credentials file",
            Self::OnboardingSession => "onboarding session",
        }
    }
}

#[derive(Clone)]
pub struct Settings {
    pub credentials: Option<Credentials>,
    pub credential_source: Option<CredentialSource>,
    pub incomplete_environment_credentials: bool,
    pub db_path: PathBuf,
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub provider: ProviderKind,
    pub catalog_url: String,
    pub catalog_refresh_interval: Duration,
    pub stock_api_url: String,
    pub stock_api_news: bool,
    pub stock_api_token: Option<SecretString>,
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
        let mut debug = formatter.debug_struct("Settings");
        debug
            .field("credentials", &self.credentials)
            .field("credential_source", &self.credential_source)
            .field(
                "incomplete_environment_credentials",
                &self.incomplete_environment_credentials,
            )
            .field("db_path", &self.db_path)
            .field("config_dir", &self.config_dir)
            .field("cache_dir", &self.cache_dir)
            .field("provider", &self.provider.id())
            .field("catalog_url", &self.catalog_url)
            .field("catalog_refresh_interval", &self.catalog_refresh_interval)
            .field("refresh_interval", &self.refresh_interval)
            .field("demo", &self.demo)
            .field("offline", &self.offline)
            .field("reset_demo", &self.reset_demo);
        match self.provider {
            ProviderKind::Alpaca => debug
                .field("data_url", &self.data_url)
                .field("trading_url", &self.trading_url)
                .field("feed", &self.feed)
                .field("request_limit_per_minute", &self.request_limit_per_minute)
                .field("snapshot_batch_size", &self.snapshot_batch_size)
                .field("history_batch_size", &self.history_batch_size)
                .finish(),
            ProviderKind::StockApi => debug
                .field("stock_api_url", &self.stock_api_url)
                .field("stock_api_news", &self.stock_api_news)
                .finish(),
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileConfig {
    provider: Option<String>,
    feed: Option<String>,
    refresh_seconds: Option<u64>,
    request_limit_per_minute: Option<u32>,
    snapshot_batch_size: Option<usize>,
    history_batch_size: Option<usize>,
    catalog_url: Option<String>,
    catalog_refresh_hours: Option<u64>,
    data_url: Option<String>,
    trading_url: Option<String>,
    providers: ProviderConfigs,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ProviderConfigs {
    alpaca: AlpacaFileConfig,
    stock_api: StockApiFileConfig,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct AlpacaFileConfig {
    feed: Option<String>,
    request_limit_per_minute: Option<u32>,
    snapshot_batch_size: Option<usize>,
    history_batch_size: Option<usize>,
    data_url: Option<String>,
    trading_url: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct StockApiFileConfig {
    base_url: Option<String>,
    news: Option<bool>,
    token: Option<String>,
}

impl Settings {
    pub fn load(cli: &Cli) -> Result<Self> {
        if let Err(error) = dotenvy::dotenv()
            && !error.not_found()
        {
            bail!("could not load local .env file");
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
        let provider_name = cli
            .provider
            .clone()
            .or_else(|| env::var("STOCK_TUI_PROVIDER").ok())
            .or_else(|| file.provider.clone())
            .unwrap_or_else(|| "alpaca".to_owned());
        let provider = ProviderKind::parse(&provider_name)?;
        let resolve_credentials = should_resolve_credentials(provider, cli.demo, cli.offline);
        let environment = if resolve_credentials {
            credentials_from_env()
        } else {
            EnvironmentCredentials {
                credentials: None,
                incomplete: false,
            }
        };
        let (credentials, credential_source) = if resolve_credentials {
            if let Some(credentials) = environment.credentials {
                (Some(credentials), Some(CredentialSource::Environment))
            } else {
                match crate::credentials::load(&config_dir.join(CREDENTIALS_FILE_NAME)) {
                    Ok(Some(credentials)) => {
                        (Some(credentials), Some(CredentialSource::StoredFile))
                    }
                    Ok(None) => (None, None),
                    Err(error) => {
                        eprintln!("Ignoring unusable stored Alpaca credentials: {error}");
                        (None, None)
                    }
                }
            }
        } else {
            (None, None)
        };
        let feed = if provider == ProviderKind::Alpaca {
            let feed = cli
                .feed
                .clone()
                .or_else(|| env::var("STOCK_TUI_FEED").ok())
                .or(file.providers.alpaca.feed)
                .or(file.feed)
                .unwrap_or_else(|| "iex".to_owned());
            if !matches!(feed.as_str(), "iex" | "sip" | "delayed_sip") {
                bail!("unsupported Alpaca feed {feed:?}; expected iex, delayed_sip, or sip");
            }
            feed
        } else {
            "managed".to_owned()
        };
        let refresh_seconds = cli
            .refresh_seconds
            .or_else(|| env_u64("STOCK_TUI_REFRESH_SECONDS"))
            .or(file.refresh_seconds)
            .unwrap_or(300)
            .clamp(30, 86_400);
        let catalog_refresh_hours = cli
            .catalog_refresh_hours
            .or_else(|| env_u64("STOCK_TUI_CATALOG_REFRESH_HOURS"))
            .or(file.catalog_refresh_hours)
            .unwrap_or(12)
            .clamp(1, 168);
        let demo = resolve_demo_mode(cli.demo);
        let db_path = cli
            .db
            .clone()
            .or_else(|| {
                env::var_os("STOCK_TUI_DB_PATH")
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
            })
            .unwrap_or_else(|| data_dir.join(default_database_name(demo)));
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).context("could not create database directory")?;
        }

        Ok(Self {
            credentials,
            credential_source,
            incomplete_environment_credentials: resolve_credentials && environment.incomplete,
            db_path,
            config_dir,
            cache_dir,
            provider,
            catalog_url: cli
                .catalog_url
                .clone()
                .or_else(|| {
                    env::var("STOCK_TUI_CATALOG_URL")
                        .ok()
                        .filter(|value| !value.trim().is_empty())
                })
                .or(file.catalog_url)
                .unwrap_or_else(|| DEFAULT_CATALOG_URL.to_owned()),
            catalog_refresh_interval: Duration::from_secs(catalog_refresh_hours * 60 * 60),
            stock_api_url: cli
                .stock_api_url
                .clone()
                .or_else(|| {
                    env::var("STOCK_TUI_STOCK_API_URL")
                        .ok()
                        .filter(|value| !value.trim().is_empty())
                })
                .or(file.providers.stock_api.base_url)
                .unwrap_or_else(|| DEFAULT_STOCK_API_URL.to_owned()),
            stock_api_news: cli
                .stock_api_news
                .or_else(|| env_bool("STOCK_TUI_STOCK_API_NEWS"))
                .or(file.providers.stock_api.news)
                .unwrap_or(true),
            stock_api_token: select_stock_api_token(
                provider,
                demo,
                cli.offline,
                env::var("STOCK_TUI_STOCK_API_TOKEN").ok(),
                file.providers.stock_api.token,
            )?,
            data_url: env::var("STOCK_TUI_DATA_URL")
                .ok()
                .or(file.providers.alpaca.data_url)
                .or(file.data_url)
                .unwrap_or_else(|| DEFAULT_DATA_URL.to_owned()),
            trading_url: env::var("STOCK_TUI_TRADING_URL")
                .ok()
                .or(file.providers.alpaca.trading_url)
                .or(file.trading_url)
                .unwrap_or_else(|| DEFAULT_TRADING_URL.to_owned()),
            feed,
            refresh_interval: Duration::from_secs(refresh_seconds),
            request_limit_per_minute: file
                .providers
                .alpaca
                .request_limit_per_minute
                .or(file.request_limit_per_minute)
                .unwrap_or(180)
                .clamp(1, 200),
            snapshot_batch_size: file
                .providers
                .alpaca
                .snapshot_batch_size
                .or(file.snapshot_batch_size)
                .unwrap_or(100)
                .clamp(1, 500),
            history_batch_size: file
                .providers
                .alpaca
                .history_batch_size
                .or(file.history_batch_size)
                .unwrap_or(50)
                .clamp(1, 200),
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
        } else if self.provider.requires_credentials() && self.credentials.is_none() {
            "setup required"
        } else {
            self.provider.display_name()
        }
    }

    #[must_use]
    pub fn credentials_path(&self) -> PathBuf {
        self.config_dir.join(CREDENTIALS_FILE_NAME)
    }

    pub fn set_credentials(&mut self, credentials: Credentials, source: CredentialSource) {
        self.credentials = Some(credentials);
        self.credential_source = Some(source);
    }
}

fn read_file_config(path: &Path) -> Result<FileConfig> {
    if !path.exists() {
        return Ok(FileConfig::default());
    }
    let contents = fs::read_to_string(path)
        .with_context(|| format!("could not read configuration at {}", path.display()))?;
    toml::from_str(&contents).map_err(|error| {
        let category = config_error_category(&error);
        if let Some(span) = error.span() {
            let (line, column) = line_and_column(&contents, span.start);
            anyhow::anyhow!(
                "invalid configuration at {}:{line}:{column} ({category})",
                path.display(),
            )
        } else {
            anyhow::anyhow!("invalid configuration at {} ({category})", path.display())
        }
    })
}

fn config_error_category(error: &toml::de::Error) -> &'static str {
    let message = error.message();
    if message.starts_with("unknown field") {
        "unknown configuration key"
    } else if message.starts_with("invalid type") {
        "wrong configuration value type"
    } else if message.starts_with("duplicate field") || message.starts_with("duplicate key") {
        "duplicate configuration key"
    } else {
        "invalid TOML syntax or value"
    }
}

fn line_and_column(contents: &str, offset: usize) -> (usize, usize) {
    let prefix = &contents.as_bytes()[..offset.min(contents.len())];
    let line = prefix.iter().filter(|byte| **byte == b'\n').count() + 1;
    let column = prefix
        .iter()
        .rev()
        .take_while(|byte| **byte != b'\n')
        .count()
        + 1;
    (line, column)
}

struct EnvironmentCredentials {
    credentials: Option<Credentials>,
    incomplete: bool,
}

fn credentials_from_env() -> EnvironmentCredentials {
    let key = env::var("ALPACA_API_KEY")
        .ok()
        .filter(|value| !value.is_empty());
    let secret = env::var("ALPACA_API_SECRET")
        .ok()
        .filter(|value| !value.is_empty());
    match (key, secret) {
        (Some(key), Some(secret)) => EnvironmentCredentials {
            credentials: Some(Credentials {
                key: SecretString::from(key),
                secret: SecretString::from(secret),
            }),
            incomplete: false,
        },
        (None, None) => EnvironmentCredentials {
            credentials: None,
            incomplete: false,
        },
        _ => EnvironmentCredentials {
            credentials: None,
            incomplete: true,
        },
    }
}

fn env_u64(key: &str) -> Option<u64> {
    env::var(key).ok()?.parse().ok()
}

fn env_bool(key: &str) -> Option<bool> {
    match env::var(key).ok()?.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn select_stock_api_token(
    provider: ProviderKind,
    demo: bool,
    offline: bool,
    environment: Option<String>,
    file: Option<String>,
) -> Result<Option<SecretString>> {
    if !should_resolve_stock_api_token(provider, demo, offline) {
        return Ok(None);
    }
    let environment = environment.filter(|value| !value.trim().is_empty());
    let file = file.filter(|value| !value.trim().is_empty());
    let selected = environment
        .map(|token| (token, "STOCK_TUI_STOCK_API_TOKEN"))
        .or_else(|| file.map(|token| (token, "providers.stock_api.token")));
    let Some((token, source)) = selected else {
        return Ok(None);
    };
    let Some(token) = validated_bearer_token(&token) else {
        bail!("invalid stock-api bearer token in {source}");
    };
    Ok(Some(SecretString::from(token.to_owned())))
}

fn should_resolve_stock_api_token(provider: ProviderKind, demo: bool, offline: bool) -> bool {
    provider == ProviderKind::StockApi && !demo && !offline
}

const fn should_resolve_credentials(provider: ProviderKind, demo: bool, offline: bool) -> bool {
    provider.requires_credentials() && !demo && !offline
}

const fn resolve_demo_mode(requested: bool) -> bool {
    requested
}

fn default_database_name(demo: bool) -> &'static str {
    if demo {
        "demo.sqlite3"
    } else {
        "market.sqlite3"
    }
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
    fn demo_mode_is_explicit() {
        assert!(!resolve_demo_mode(false));
        assert!(resolve_demo_mode(true));
    }

    #[test]
    fn default_trading_url_uses_alpaca_paper_endpoint() {
        assert_eq!(DEFAULT_TRADING_URL, "https://paper-api.alpaca.markets");
    }

    #[test]
    fn provider_names_are_strict_and_case_insensitive() {
        assert_eq!(
            ProviderKind::parse("ALPACA").expect("provider"),
            ProviderKind::Alpaca
        );
        assert_eq!(
            ProviderKind::parse("stock-api").expect("provider"),
            ProviderKind::StockApi
        );
        assert_eq!(
            ProviderKind::parse("STOCK_API").expect("provider alias"),
            ProviderKind::StockApi
        );
        assert!(ProviderKind::parse("unknown").is_err());
    }

    #[test]
    fn provider_specific_toml_is_namespaced() {
        use secrecy::ExposeSecret;

        let file: FileConfig = toml::from_str(
            r#"
provider = "alpaca"
catalog_refresh_hours = 24

[providers.alpaca]
feed = "delayed_sip"
history_batch_size = 75

[providers.stock_api]
base_url = "http://127.0.0.1:8787"
news = false
token = "private-development-token"
"#,
        )
        .expect("provider configuration");

        assert_eq!(file.provider.as_deref(), Some("alpaca"));
        assert_eq!(file.catalog_refresh_hours, Some(24));
        assert_eq!(file.providers.alpaca.feed.as_deref(), Some("delayed_sip"));
        assert_eq!(file.providers.alpaca.history_batch_size, Some(75));
        assert_eq!(
            file.providers.stock_api.base_url.as_deref(),
            Some("http://127.0.0.1:8787")
        );
        assert_eq!(file.providers.stock_api.news, Some(false));
        let token = select_stock_api_token(
            ProviderKind::StockApi,
            false,
            false,
            None,
            file.providers.stock_api.token,
        )
        .expect("valid token");
        assert_eq!(
            token.as_ref().map(ExposeSecret::expose_secret),
            Some("private-development-token")
        );
    }

    #[test]
    fn example_configuration_matches_the_strict_schema() {
        toml::from_str::<FileConfig>(include_str!("../config.example.toml"))
            .expect("config.example.toml");
    }

    #[test]
    fn stock_api_token_precedence_and_inactive_modes_are_explicit() {
        use secrecy::ExposeSecret;

        let selected = select_stock_api_token(
            ProviderKind::StockApi,
            false,
            false,
            Some("environment-token".to_owned()),
            Some("file-token".to_owned()),
        )
        .expect("valid selection")
        .expect("environment token");
        assert_eq!(selected.expose_secret(), "environment-token");

        let selected = select_stock_api_token(
            ProviderKind::StockApi,
            false,
            false,
            Some("  ".to_owned()),
            Some("  file-token  ".to_owned()),
        )
        .expect("valid selection")
        .expect("file token");
        assert_eq!(selected.expose_secret(), "file-token");

        let invalid_environment = "invalid environment token";
        let error = select_stock_api_token(
            ProviderKind::StockApi,
            false,
            false,
            Some(invalid_environment.to_owned()),
            Some("valid-file-token".to_owned()),
        )
        .expect_err("an invalid higher-precedence token must not fall back");
        let rendered = format!("{error:#}");
        assert!(rendered.contains("STOCK_TUI_STOCK_API_TOKEN"));
        assert!(!rendered.contains(invalid_environment));

        for (provider, demo, offline) in [
            (ProviderKind::Alpaca, false, false),
            (ProviderKind::StockApi, true, false),
            (ProviderKind::StockApi, false, true),
        ] {
            assert!(
                select_stock_api_token(
                    provider,
                    demo,
                    offline,
                    Some("environment-token".to_owned()),
                    Some("file-token".to_owned()),
                )
                .expect("inactive modes ignore tokens")
                .is_none()
            );
        }
    }

    #[test]
    fn malformed_configuration_and_invalid_tokens_never_leak_secrets() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let path = temp.path().join("config.toml");
        let secret = "do-not-echo-this-private-token";
        fs::write(
            &path,
            format!(
                r#"
provider = "stock-api"
[providers.stock_api]
token = "{secret}
"#
            ),
        )
        .expect("malformed configuration");

        let error = match read_file_config(&path) {
            Ok(_) => panic!("malformed TOML must fail"),
            Err(error) => error,
        };
        let rendered = format!("{error:#}");
        assert!(rendered.contains("invalid configuration at"));
        assert!(rendered.contains("config.toml:"));
        assert!(rendered.contains("invalid TOML syntax or value"));
        assert!(!rendered.contains(secret));

        for (contents, category) in [
            (
                "unexpected_private_key = \"secret\"\n",
                "unknown configuration key",
            ),
            (
                "refresh_seconds = \"secret\"\n",
                "wrong configuration value type",
            ),
        ] {
            fs::write(&path, contents).expect("invalid configuration fixture");
            let error = match read_file_config(&path) {
                Ok(_) => panic!("configuration must fail"),
                Err(error) => error,
            };
            let rendered = format!("{error:#}");
            assert!(rendered.contains(category));
            assert!(!rendered.contains("secret"));
            assert!(!rendered.contains("unexpected_private_key"));
        }

        for invalid in [
            format!("{secret}\nnext-line"),
            "x".repeat(crate::providers::stock_api::MAX_BEARER_TOKEN_LENGTH + 1),
            format!("{secret} internal-space"),
        ] {
            let error =
                select_stock_api_token(ProviderKind::StockApi, false, false, None, Some(invalid))
                    .expect_err("invalid token");
            let rendered = format!("{error:#}");
            assert!(rendered.contains("providers.stock_api.token"));
            assert!(!rendered.contains(secret));
        }
    }

    #[test]
    fn default_catalog_uses_the_static_cloudflare_hostname() {
        assert_eq!(
            DEFAULT_CATALOG_URL,
            "https://stock.chatcode.dev/catalog/sec-catalog.json"
        );
    }

    #[test]
    fn stock_api_uses_separate_optional_authentication() {
        assert!(!ProviderKind::StockApi.requires_credentials());
        assert!(ProviderKind::Alpaca.requires_credentials());
        assert_eq!(DEFAULT_STOCK_API_URL, "https://stock.chatcode.dev/api");
        assert!(!should_resolve_credentials(
            ProviderKind::StockApi,
            false,
            false
        ));
        assert!(should_resolve_credentials(
            ProviderKind::Alpaca,
            false,
            false
        ));
        assert!(should_resolve_stock_api_token(
            ProviderKind::StockApi,
            false,
            false
        ));
        assert!(!should_resolve_stock_api_token(
            ProviderKind::Alpaca,
            false,
            false
        ));
        assert!(!should_resolve_stock_api_token(
            ProviderKind::StockApi,
            true,
            false
        ));
        assert!(!should_resolve_stock_api_token(
            ProviderKind::StockApi,
            false,
            true
        ));
    }

    #[test]
    fn stock_api_debug_and_mode_do_not_leak_alpaca_configuration() {
        let settings = Settings {
            credentials: None,
            credential_source: None,
            incomplete_environment_credentials: false,
            db_path: PathBuf::from("market.sqlite3"),
            config_dir: PathBuf::from("config"),
            cache_dir: PathBuf::from("cache"),
            provider: ProviderKind::StockApi,
            catalog_url: DEFAULT_CATALOG_URL.to_owned(),
            catalog_refresh_interval: Duration::from_secs(12 * 60 * 60),
            stock_api_url: "http://127.0.0.1:8787".to_owned(),
            stock_api_news: false,
            stock_api_token: Some(SecretString::from("private-test-token".to_owned())),
            data_url: DEFAULT_DATA_URL.to_owned(),
            trading_url: DEFAULT_TRADING_URL.to_owned(),
            feed: "iex".to_owned(),
            refresh_interval: Duration::from_secs(300),
            request_limit_per_minute: 180,
            snapshot_batch_size: 100,
            history_batch_size: 50,
            demo: false,
            offline: false,
            reset_demo: false,
        };

        let rendered = format!("{settings:#?}");
        assert_eq!(settings.mode_label(), "Stock API");
        assert!(rendered.contains("stock_api_url"));
        assert!(rendered.contains("stock_api_news"));
        assert!(!rendered.contains("private-test-token"));
        assert!(!rendered.contains("stock_api_token"));
        assert!(!rendered.contains("data.alpaca.markets"));
        assert!(!rendered.contains("paper-api.alpaca.markets"));
        assert!(!rendered.contains("feed:"));
    }

    #[test]
    fn demo_and_live_modes_have_separate_default_databases() {
        assert_eq!(default_database_name(true), "demo.sqlite3");
        assert_eq!(default_database_name(false), "market.sqlite3");
    }
}
