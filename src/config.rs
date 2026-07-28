use std::{
    env, fmt, fs,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use anyhow::{Context, Result, bail};
use directories::ProjectDirs;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use toml_edit::{DocumentMut, value};

use crate::{cli::Cli, providers::stock_api::validated_bearer_token};

const DEFAULT_DATA_URL: &str = "https://data.alpaca.markets";
const DEFAULT_TRADING_URL: &str = "https://paper-api.alpaca.markets";
pub const DEFAULT_STOCK_API_URL: &str = "https://stock.chatcode.dev/api";
pub const DEFAULT_CATALOG_URL: &str = "https://stock.chatcode.dev/catalog/sec-catalog.json";
pub const CONFIG_FILE_NAME: &str = "config.toml";
pub const CREDENTIALS_FILE_NAME: &str = "credentials.env";
const MAX_ALPACA_CREDENTIAL_LENGTH: usize = 4_096;

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
    ConfigFile,
    StoredFile,
    OnboardingSession,
}

impl CredentialSource {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::ConfigFile => "config.toml",
            Self::StoredFile => "legacy credentials file",
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
    api_key: Option<toml::Value>,
    api_secret: Option<toml::Value>,
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

        let config_path = config_dir.join(CONFIG_FILE_NAME);
        let file = read_file_config(&config_path)?;
        let environment_provider = if cli.provider.is_none() {
            environment_value("STOCK_TUI_PROVIDER")?
        } else {
            None
        };
        let provider = select_provider(
            cli.provider.as_deref(),
            environment_provider.as_deref(),
            file.provider.as_deref(),
        )?;
        // A saved Alpaca pair is considered only after provider precedence is resolved.
        let resolve_credentials = should_resolve_credentials(provider, cli.demo, cli.offline);
        let environment = if resolve_credentials {
            credentials_from_env()
        } else {
            EnvironmentCredentials {
                credentials: None,
                incomplete: false,
            }
        };
        let file_credentials = if resolve_credentials {
            alpaca_credentials_after_environment(&environment, &file.providers.alpaca)?
        } else {
            None
        };
        let (credentials, credential_source) = if resolve_credentials {
            let stored_credentials =
                if environment.credentials.is_none() && file_credentials.is_none() {
                    match crate::credentials::load(&config_dir.join(CREDENTIALS_FILE_NAME)) {
                        Ok(credentials) => credentials,
                        Err(error) => {
                            eprintln!("Ignoring unusable legacy Alpaca credentials: {error}");
                            None
                        }
                    }
                } else {
                    None
                };
            select_alpaca_credentials(
                environment.credentials,
                file_credentials,
                stored_credentials,
            )
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
        let refresh_seconds = match cli.refresh_seconds {
            Some(value) => value,
            None => env_u64("STOCK_TUI_REFRESH_SECONDS")?
                .or(file.refresh_seconds)
                .unwrap_or(300),
        }
        .clamp(30, 86_400);
        let catalog_refresh_hours = match cli.catalog_refresh_hours {
            Some(value) => value,
            None => env_u64("STOCK_TUI_CATALOG_REFRESH_HOURS")?
                .or(file.catalog_refresh_hours)
                .unwrap_or(12),
        }
        .clamp(1, 168);
        let stock_api_news = match cli.stock_api_news {
            Some(value) => value,
            None => env_bool("STOCK_TUI_STOCK_API_NEWS")?
                .or(file.providers.stock_api.news)
                .unwrap_or(true),
        };
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
            stock_api_news,
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

    #[must_use]
    pub fn config_path(&self) -> PathBuf {
        self.config_dir.join(CONFIG_FILE_NAME)
    }

    pub fn config_credentials(&self) -> Result<Option<Credentials>> {
        let file = read_file_config(&self.config_path())?;
        alpaca_credentials_from_file(&file.providers.alpaca)
    }

    pub fn save_credentials_to_config(&self, credentials: &Credentials) -> Result<()> {
        save_alpaca_credentials(&self.config_path(), credentials)
    }

    pub fn set_credentials(&mut self, credentials: Credentials, source: CredentialSource) {
        self.credentials = Some(credentials);
        self.credential_source = Some(source);
    }
}

fn alpaca_credentials_from_file(config: &AlpacaFileConfig) -> Result<Option<Credentials>> {
    let key = alpaca_credential_text(config.api_key.as_ref())?;
    let secret = alpaca_credential_text(config.api_secret.as_ref())?;
    match (key, secret) {
        (None, None) => Ok(None),
        (Some(key), Some(secret)) => Ok(Some(Credentials {
            key: SecretString::from(key.to_owned()),
            secret: SecretString::from(secret.to_owned()),
        })),
        _ => bail!(
            "providers.alpaca.api_key and providers.alpaca.api_secret must be configured together"
        ),
    }
}

fn alpaca_credential_text(value: Option<&toml::Value>) -> Result<Option<&str>> {
    match value {
        None => Ok(None),
        Some(value) => match value.as_str() {
            Some("") => Ok(None),
            Some(value) if valid_alpaca_credential(value) => Ok(Some(value)),
            _ => bail!("invalid Alpaca credential value in providers.alpaca (values redacted)"),
        },
    }
}

fn alpaca_credentials_after_environment(
    environment: &EnvironmentCredentials,
    config: &AlpacaFileConfig,
) -> Result<Option<Credentials>> {
    if environment.credentials.is_some() {
        Ok(None)
    } else {
        alpaca_credentials_from_file(config)
    }
}

fn valid_alpaca_credential(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ALPACA_CREDENTIAL_LENGTH
        && !value
            .chars()
            .any(|character| matches!(character, '\0' | '\r' | '\n'))
}

fn select_alpaca_credentials(
    environment: Option<Credentials>,
    file: Option<Credentials>,
    stored: Option<Credentials>,
) -> (Option<Credentials>, Option<CredentialSource>) {
    environment
        .map(|credentials| (Some(credentials), Some(CredentialSource::Environment)))
        .or_else(|| file.map(|credentials| (Some(credentials), Some(CredentialSource::ConfigFile))))
        .or_else(|| {
            stored.map(|credentials| (Some(credentials), Some(CredentialSource::StoredFile)))
        })
        .unwrap_or((None, None))
}

fn save_alpaca_credentials(path: &Path, credentials: &Credentials) -> Result<()> {
    let key = credentials.key.expose_secret();
    let secret = credentials.secret.expose_secret();
    if !valid_alpaca_credential(key) || !valid_alpaca_credential(secret) {
        bail!("could not store invalid Alpaca credentials (values redacted)");
    }
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => bail!("could not read existing configuration before saving credentials"),
    };
    let mut document = if contents.trim().is_empty() {
        DocumentMut::new()
    } else {
        contents
            .parse::<DocumentMut>()
            .map_err(|_| anyhow::anyhow!("could not update invalid configuration"))?
    };
    document["providers"]["alpaca"]["api_key"] = value(key);
    document["providers"]["alpaca"]["api_secret"] = value(secret);
    let rendered = document.to_string();
    toml::from_str::<FileConfig>(&rendered)
        .map_err(|_| anyhow::anyhow!("could not validate updated configuration"))?;
    write_private_config(path, rendered.as_bytes())
}

fn write_private_config(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("could not create configuration directory")?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(path)
        .context("could not open configuration for writing")?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .context("could not restrict configuration permissions")?;
    file.write_all(contents)
        .and_then(|()| file.sync_all())
        .context("could not write configuration")?;
    Ok(())
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

fn environment_value(key: &str) -> Result<Option<String>> {
    match env::var(key) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => bail!("invalid non-Unicode value in {key}"),
    }
}

fn env_u64(key: &str) -> Result<Option<u64>> {
    parse_env_u64(key, environment_value(key)?)
}

fn parse_env_u64(key: &str, value: Option<String>) -> Result<Option<u64>> {
    let Some(value) = value else {
        return Ok(None);
    };
    value
        .parse()
        .map(Some)
        .map_err(|_| anyhow::anyhow!("invalid unsigned integer in {key}"))
}

fn env_bool(key: &str) -> Result<Option<bool>> {
    parse_env_bool(key, environment_value(key)?)
}

fn parse_env_bool(key: &str, value: Option<String>) -> Result<Option<bool>> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(Some(true)),
        "0" | "false" | "no" | "off" => Ok(Some(false)),
        _ => bail!("invalid boolean in {key}"),
    }
}

fn select_provider(
    cli: Option<&str>,
    environment: Option<&str>,
    file: Option<&str>,
) -> Result<ProviderKind> {
    let selected = cli.or(environment).or(file).unwrap_or("alpaca");
    ProviderKind::parse(selected)
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

    fn credentials(key: &str, secret: &str) -> Credentials {
        Credentials {
            key: SecretString::from(key.to_owned()),
            secret: SecretString::from(secret.to_owned()),
        }
    }

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
    fn provider_precedence_is_independent_of_managed_alpaca_credentials() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let credentials_path = temp.path().join(CREDENTIALS_FILE_NAME);
        crate::credentials::save(
            &credentials_path,
            &Credentials {
                key: SecretString::from("managed-key".to_owned()),
                secret: SecretString::from("managed-secret".to_owned()),
            },
        )
        .expect("managed credentials");
        assert!(
            crate::credentials::load(&credentials_path)
                .expect("read managed credentials")
                .is_some()
        );

        let file: FileConfig =
            toml::from_str(r#"provider = "stock-api""#).expect("provider configuration");
        let provider =
            select_provider(None, None, file.provider.as_deref()).expect("file-selected provider");
        assert_eq!(provider, ProviderKind::StockApi);
        assert!(!should_resolve_credentials(provider, false, false));

        assert_eq!(
            select_provider(None, Some("alpaca"), Some("stock-api"))
                .expect("environment-selected provider"),
            ProviderKind::Alpaca
        );
        assert_eq!(
            select_provider(Some("stock-api"), Some("alpaca"), Some("alpaca"))
                .expect("CLI-selected provider"),
            ProviderKind::StockApi
        );
        assert_eq!(
            select_provider(None, None, None).expect("default provider"),
            ProviderKind::Alpaca
        );
        assert!(select_provider(Some("unknown"), Some("stock-api"), Some("stock-api")).is_err());
    }

    #[test]
    fn malformed_typed_environment_values_never_fall_through_or_echo() {
        let invalid = "do-not-echo-this-value";
        let error = parse_env_u64("STOCK_TUI_REFRESH_SECONDS", Some(invalid.to_owned()))
            .expect_err("invalid integer");
        let rendered = format!("{error:#}");
        assert!(rendered.contains("STOCK_TUI_REFRESH_SECONDS"));
        assert!(!rendered.contains(invalid));

        let error = parse_env_bool("STOCK_TUI_STOCK_API_NEWS", Some(invalid.to_owned()))
            .expect_err("invalid boolean");
        let rendered = format!("{error:#}");
        assert!(rendered.contains("STOCK_TUI_STOCK_API_NEWS"));
        assert!(!rendered.contains(invalid));

        assert_eq!(
            parse_env_u64("STOCK_TUI_REFRESH_SECONDS", Some("600".to_owned()))
                .expect("valid integer"),
            Some(600)
        );
        assert_eq!(
            parse_env_bool("STOCK_TUI_STOCK_API_NEWS", Some("yes".to_owned()))
                .expect("valid boolean"),
            Some(true)
        );
    }

    #[test]
    fn provider_specific_toml_is_namespaced() {
        use secrecy::ExposeSecret;

        let file: FileConfig = toml::from_str(
            r#"
provider = "alpaca"
catalog_refresh_hours = 24

[providers.alpaca]
api_key = "toml-key"
api_secret = "toml-secret"
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
        let credentials =
            alpaca_credentials_from_file(&file.providers.alpaca).expect("TOML credentials");
        let credentials = credentials.expect("complete TOML pair");
        assert_eq!(credentials.key.expose_secret(), "toml-key");
        assert_eq!(credentials.secret.expose_secret(), "toml-secret");
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
    fn alpaca_credentials_follow_environment_toml_legacy_precedence() {
        use secrecy::ExposeSecret;

        let (selected, source) = select_alpaca_credentials(
            Some(credentials("environment-key", "environment-secret")),
            Some(credentials("toml-key", "toml-secret")),
            Some(credentials("legacy-key", "legacy-secret")),
        );
        assert_eq!(source, Some(CredentialSource::Environment));
        assert_eq!(
            selected
                .as_ref()
                .expect("environment credentials")
                .key
                .expose_secret(),
            "environment-key"
        );

        let (selected, source) = select_alpaca_credentials(
            None,
            Some(credentials("toml-key", "toml-secret")),
            Some(credentials("legacy-key", "legacy-secret")),
        );
        assert_eq!(source, Some(CredentialSource::ConfigFile));
        assert_eq!(
            selected
                .as_ref()
                .expect("TOML credentials")
                .secret
                .expose_secret(),
            "toml-secret"
        );

        let (selected, source) =
            select_alpaca_credentials(None, None, Some(credentials("legacy-key", "legacy-secret")));
        assert_eq!(source, Some(CredentialSource::StoredFile));
        assert_eq!(
            selected
                .as_ref()
                .expect("legacy credentials")
                .key
                .expose_secret(),
            "legacy-key"
        );
    }

    #[test]
    fn incomplete_or_invalid_toml_credentials_are_redacted() {
        let incomplete = AlpacaFileConfig {
            api_key: Some(toml::Value::String("do-not-echo-key".to_owned())),
            ..AlpacaFileConfig::default()
        };
        let error = alpaca_credentials_from_file(&incomplete)
            .expect_err("incomplete TOML credentials must fail");
        let rendered = format!("{error:#}");
        assert!(rendered.contains("must be configured together"));
        assert!(!rendered.contains("do-not-echo-key"));

        let invalid_secret = "do-not-echo-secret\nnext-line";
        let invalid = AlpacaFileConfig {
            api_key: Some(toml::Value::String("valid-key".to_owned())),
            api_secret: Some(toml::Value::String(invalid_secret.to_owned())),
            ..AlpacaFileConfig::default()
        };
        let error =
            alpaca_credentials_from_file(&invalid).expect_err("invalid TOML credentials must fail");
        let rendered = format!("{error:#}");
        assert!(rendered.contains("values redacted"));
        assert!(!rendered.contains("valid-key"));
        assert!(!rendered.contains(invalid_secret));
    }

    #[test]
    fn complete_environment_pair_skips_lower_priority_toml_pair_validation() {
        let environment = EnvironmentCredentials {
            credentials: Some(credentials("environment-key", "environment-secret")),
            incomplete: false,
        };
        let incomplete_file = AlpacaFileConfig {
            api_key: Some(toml::Value::String("lower-priority-key".to_owned())),
            ..AlpacaFileConfig::default()
        };

        assert!(
            alpaca_credentials_after_environment(&environment, &incomplete_file)
                .expect("lower-priority pair is ignored")
                .is_none()
        );
        let absent_environment = EnvironmentCredentials {
            credentials: None,
            incomplete: false,
        };
        assert!(
            alpaca_credentials_after_environment(&absent_environment, &incomplete_file).is_err()
        );

        let malformed_file = AlpacaFileConfig {
            api_key: Some(toml::Value::Integer(123)),
            api_secret: Some(toml::Value::String("lower-priority-secret".to_owned())),
            ..AlpacaFileConfig::default()
        };
        assert!(
            alpaca_credentials_after_environment(&environment, &malformed_file)
                .expect("malformed lower-priority pair is ignored")
                .is_none()
        );
        assert!(
            alpaca_credentials_after_environment(&absent_environment, &malformed_file).is_err()
        );
    }

    #[test]
    fn saving_alpaca_credentials_preserves_toml_comments_and_settings() {
        use secrecy::ExposeSecret;

        let temp = tempfile::TempDir::new().expect("temp dir");
        let path = temp.path().join(CONFIG_FILE_NAME);
        fs::write(
            &path,
            r#"# keep this operator comment
provider = "alpaca"

[providers.alpaca]
feed = "delayed_sip" # keep this feed comment

[providers.stock_api]
base_url = "http://127.0.0.1:8787"
"#,
        )
        .expect("existing configuration");
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
            .expect("loose initial permissions");

        save_alpaca_credentials(&path, &credentials("persisted-key", "persisted-secret"))
            .expect("save TOML credentials");

        let rendered = fs::read_to_string(&path).expect("updated configuration");
        assert!(rendered.contains("# keep this operator comment"));
        assert!(rendered.contains("feed = \"delayed_sip\" # keep this feed comment"));
        assert!(rendered.contains("base_url = \"http://127.0.0.1:8787\""));
        let file = read_file_config(&path).expect("strict updated configuration");
        let stored = alpaca_credentials_from_file(&file.providers.alpaca)
            .expect("stored pair")
            .expect("complete pair");
        assert_eq!(stored.key.expose_secret(), "persisted-key");
        assert_eq!(stored.secret.expose_secret(), "persisted-secret");

        #[cfg(unix)]
        {
            let mode = fs::metadata(&path)
                .expect("configuration metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn saving_toml_credentials_never_echoes_invalid_values() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let path = temp.path().join(CONFIG_FILE_NAME);
        let secret = "do-not-echo-this-secret\nnext-line";
        let error = save_alpaca_credentials(&path, &credentials("key", secret))
            .expect_err("invalid credentials must not be written");
        let rendered = format!("{error:#}");
        assert!(rendered.contains("values redacted"));
        assert!(!rendered.contains(secret));
        assert!(!path.exists());
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
