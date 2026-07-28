//! Provider-neutral HTTP adapter for a separately hosted stock API.

use std::{collections::HashSet, fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::{Client, Response, StatusCode, Url, header::HeaderMap};
use serde::{Deserialize, de::DeserializeOwned};

use super::{AssetProvider, MarketDataProvider, NewsProvider, ProviderError, ProviderSet};
use crate::domain::{Bar, Company, NewsItem, Snapshot};

const API_SCHEMA_VERSION: u16 = 1;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_MAX_RETRIES: usize = 3;
const DEFAULT_RETRY_BASE: Duration = Duration::from_millis(250);
const DEFAULT_MAX_RETRY_DELAY: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
const MAX_PAGES: usize = 100;
const MAX_ASSETS: usize = 100_000;
const MAX_BARS: usize = 1_000_000;
const MAX_SNAPSHOT_SYMBOLS: usize = 100;
const MAX_HISTORY_SYMBOLS: usize = 50;
const MAX_NEWS_ITEMS: usize = 50;
const MAX_ERROR_LENGTH: usize = 240;
const MAX_NAME_LENGTH: usize = 512;
const MAX_HEADLINE_LENGTH: usize = 2_048;
const MAX_SUMMARY_LENGTH: usize = 16_384;

#[derive(Debug, Clone, Copy)]
struct RetryPolicy {
    max_retries: usize,
    base_delay: Duration,
    max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
            base_delay: DEFAULT_RETRY_BASE,
            max_delay: DEFAULT_MAX_RETRY_DELAY,
        }
    }
}

impl RetryPolicy {
    fn delay(self, attempt: usize, headers: Option<&HeaderMap>) -> Duration {
        let retry_after = headers
            .and_then(|values| values.get(reqwest::header::RETRY_AFTER))
            .and_then(|value| value.to_str().ok())
            .and_then(parse_retry_after)
            .map(|delay| delay.min(self.max_delay));
        retry_after.unwrap_or_else(|| {
            let exponent = u32::try_from(attempt).unwrap_or(u32::MAX).min(16);
            self.base_delay
                .saturating_mul(2_u32.saturating_pow(exponent))
                .min(self.max_delay)
        })
    }
}

/// Client for the stock-api v1 JSON contract.
pub struct StockApiProvider {
    client: Client,
    base_url: Url,
    news_enabled: bool,
    retry: RetryPolicy,
}

impl fmt::Debug for StockApiProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StockApiProvider")
            .field("base_url", &self.base_url)
            .field("news_enabled", &self.news_enabled)
            .finish_non_exhaustive()
    }
}

impl StockApiProvider {
    pub const ID: &'static str = "stock-api";
    pub const DISPLAY_NAME: &'static str = "Stock API";

    pub fn new(base_url: &str, news_enabled: bool) -> Result<Self, ProviderError> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .user_agent(concat!("stock-tui/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| ProviderError::Transport {
                kind: "client setup",
            })?;
        Self::with_client(base_url, news_enabled, client)
    }

    /// Construct with a controlled HTTP client, primarily for local fixtures.
    pub fn with_client(
        base_url: &str,
        news_enabled: bool,
        client: Client,
    ) -> Result<Self, ProviderError> {
        Ok(Self {
            client,
            base_url: validate_base_url(base_url)?,
            news_enabled,
            retry: RetryPolicy::default(),
        })
    }

    /// Convert this adapter into the provider-neutral runtime facade.
    #[must_use]
    pub fn into_provider_set(self) -> ProviderSet {
        let news_enabled = self.news_enabled;
        let provider = Arc::new(self);
        let assets: Arc<dyn AssetProvider> = provider.clone();
        let market_data: Arc<dyn MarketDataProvider> = provider.clone();
        let providers = ProviderSet::new(Self::ID, Self::DISPLAY_NAME, assets, market_data);
        if news_enabled {
            let news: Arc<dyn NewsProvider> = provider;
            providers.with_news(news)
        } else {
            providers
        }
    }

    fn endpoint(&self, path: &str) -> Result<Url, ProviderError> {
        self.base_url
            .join(path)
            .map_err(|_| ProviderError::InvalidRequest("invalid stock API endpoint".to_owned()))
    }

    async fn get_page<T>(
        &self,
        path: &str,
        query: &[(String, String)],
        resource: &'static str,
    ) -> Result<ApiPage<T>, ProviderError>
    where
        T: DeserializeOwned,
    {
        let url = self.endpoint(path)?;
        for attempt in 0..=self.retry.max_retries {
            let response = self
                .client
                .get(url.clone())
                .header(reqwest::header::ACCEPT, "application/json")
                .query(query)
                .send()
                .await;
            let response = match response {
                Ok(response) => response,
                Err(error) if attempt < self.retry.max_retries => {
                    tokio::time::sleep(self.retry.delay(attempt, None)).await;
                    drop(error);
                    continue;
                }
                Err(error) => {
                    return Err(ProviderError::Transport {
                        kind: transport_error_kind(&error),
                    });
                }
            };

            let status = response.status();
            if status.is_success() {
                let body = read_bounded_body(response, resource).await?;
                let page: ApiPage<T> = serde_json::from_slice(&body)
                    .map_err(|_| ProviderError::InvalidData { resource })?;
                if page.schema_version != API_SCHEMA_VERSION {
                    return Err(ProviderError::InvalidData { resource });
                }
                return Ok(page);
            }

            if retryable_status(status) && attempt < self.retry.max_retries {
                let delay = self.retry.delay(attempt, Some(response.headers()));
                drop(response);
                tokio::time::sleep(delay).await;
                continue;
            }
            return Err(response_error(response).await);
        }

        Err(ProviderError::Transport {
            kind: "retry exhaustion",
        })
    }

    async fn paginated<T>(
        &self,
        path: &str,
        query: &[(String, String)],
        resource: &'static str,
        max_items: usize,
    ) -> Result<Vec<T>, ProviderError>
    where
        T: DeserializeOwned,
    {
        let mut query = query.to_vec();
        let mut results = Vec::new();
        let mut seen_tokens = HashSet::new();
        for _ in 0..MAX_PAGES {
            let page: ApiPage<T> = self.get_page(path, &query, resource).await?;
            if results.len().saturating_add(page.data.len()) > max_items {
                return Err(ProviderError::InvalidData { resource });
            }
            results.extend(page.data);
            let Some(token) = page
                .next_page_token
                .filter(|token| !token.trim().is_empty())
            else {
                return Ok(results);
            };
            if !seen_tokens.insert(token.clone()) {
                return Err(ProviderError::InvalidData { resource });
            }
            query.retain(|(key, _)| key != "page_token");
            query.push(("page_token".to_owned(), token));
        }
        Err(ProviderError::InvalidData { resource })
    }
}

#[async_trait]
impl AssetProvider for StockApiProvider {
    async fn fetch_assets(&self) -> Result<Vec<Company>, ProviderError> {
        let assets: Vec<AssetDto> = self
            .paginated(
                "v1/assets",
                &[("status".to_owned(), "active".to_owned())],
                "asset",
                MAX_ASSETS,
            )
            .await?;
        let updated_at = Utc::now();
        let mut companies = assets
            .into_iter()
            .map(|asset| asset.into_domain(updated_at))
            .collect::<Result<Vec<_>, _>>()?;
        companies.sort_unstable_by(|left, right| left.symbol.cmp(&right.symbol));
        companies.dedup_by(|left, right| left.symbol == right.symbol);
        Ok(companies)
    }
}

#[async_trait]
impl MarketDataProvider for StockApiProvider {
    async fn fetch_bars(
        &self,
        symbols: &[String],
        timeframe: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Bar>, ProviderError> {
        let timeframe = timeframe.trim();
        if timeframe.is_empty() {
            return Err(ProviderError::InvalidRequest(
                "timeframe must not be empty".to_owned(),
            ));
        }
        if end <= start {
            return Err(ProviderError::InvalidRequest(
                "bar end must follow start".to_owned(),
            ));
        }
        let symbols = normalize_symbols(symbols)?;
        if symbols.is_empty() {
            return Ok(Vec::new());
        }

        let mut result = Vec::new();
        for batch in symbols.chunks(MAX_HISTORY_SYMBOLS) {
            let requested = batch.iter().cloned().collect::<HashSet<_>>();
            let query = vec![
                ("symbols".to_owned(), batch.join(",")),
                ("timeframe".to_owned(), timeframe.to_owned()),
                (
                    "start".to_owned(),
                    start.to_rfc3339_opts(SecondsFormat::Secs, true),
                ),
                (
                    "end".to_owned(),
                    end.to_rfc3339_opts(SecondsFormat::Secs, true),
                ),
                ("adjustment".to_owned(), "all".to_owned()),
            ];
            let bars: Vec<BarDto> = self.paginated("v1/bars", &query, "bar", MAX_BARS).await?;
            result.extend(
                bars.into_iter()
                    .map(|bar| bar.into_domain(timeframe, &requested))
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
        result.sort_unstable_by(|left, right| {
            left.symbol
                .cmp(&right.symbol)
                .then(left.timestamp.cmp(&right.timestamp))
        });
        Ok(result)
    }

    async fn fetch_snapshots(&self, symbols: &[String]) -> Result<Vec<Snapshot>, ProviderError> {
        let symbols = normalize_symbols(symbols)?;
        if symbols.is_empty() {
            return Ok(Vec::new());
        }
        let mut result = Vec::new();
        for batch in symbols.chunks(MAX_SNAPSHOT_SYMBOLS) {
            let requested = batch.iter().cloned().collect::<HashSet<_>>();
            let page: ApiPage<SnapshotDto> = self
                .get_page(
                    "v1/snapshots",
                    &[("symbols".to_owned(), batch.join(","))],
                    "snapshot",
                )
                .await?;
            if page
                .next_page_token
                .is_some_and(|token| !token.trim().is_empty())
                || page.data.len() > batch.len()
            {
                return Err(ProviderError::InvalidData {
                    resource: "snapshot",
                });
            }
            result.extend(
                page.data
                    .into_iter()
                    .map(|snapshot| snapshot.into_domain(&requested))
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
        result.sort_unstable_by(|left, right| left.symbol.cmp(&right.symbol));
        result.dedup_by(|left, right| left.symbol == right.symbol);
        Ok(result)
    }
}

#[async_trait]
impl NewsProvider for StockApiProvider {
    async fn fetch_news(
        &self,
        symbols: &[String],
        limit: usize,
    ) -> Result<Vec<NewsItem>, ProviderError> {
        let symbols = normalize_symbols(symbols)?;
        if symbols.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(MAX_NEWS_ITEMS);
        let page: ApiPage<NewsDto> = self
            .get_page(
                "v1/news",
                &[
                    ("symbols".to_owned(), symbols.join(",")),
                    ("limit".to_owned(), limit.to_string()),
                ],
                "news",
            )
            .await?;
        if page
            .next_page_token
            .is_some_and(|token| !token.trim().is_empty())
        {
            return Err(ProviderError::InvalidData { resource: "news" });
        }
        let mut items = page
            .data
            .into_iter()
            .map(NewsDto::into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        items.sort_unstable_by_key(|item| std::cmp::Reverse(item.published_at));
        items.truncate(limit);
        Ok(items)
    }
}

#[derive(Debug, Deserialize)]
struct ApiPage<T> {
    schema_version: u16,
    data: Vec<T>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: ErrorDto,
}

#[derive(Debug, Deserialize)]
struct ErrorDto {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize)]
struct AssetDto {
    symbol: String,
    name: String,
    exchange: String,
    #[serde(default)]
    market_cap: Option<f64>,
    #[serde(default)]
    updated_at: Option<DateTime<Utc>>,
}

impl AssetDto {
    fn into_domain(self, fetched_at: DateTime<Utc>) -> Result<Company, ProviderError> {
        let symbol = normalize_symbol(&self.symbol, "asset")?;
        let name = validate_text(self.name, MAX_NAME_LENGTH, "asset")?;
        let exchange = validate_text(self.exchange, MAX_NAME_LENGTH, "asset")?;
        if !valid_optional_positive(self.market_cap) {
            return Err(ProviderError::InvalidData { resource: "asset" });
        }
        Ok(Company {
            symbol,
            name,
            sector: None,
            raw_sector: None,
            exchange,
            industry: String::new(),
            market_cap: self.market_cap,
            size_proxy: None,
            size_proxy_source: None,
            size_proxy_as_of: None,
            size_proxy_confidence: None,
            shares_outstanding: None,
            shares_source: None,
            shares_as_of: None,
            shares_method: None,
            shares_confidence: None,
            rank: None,
            description: String::new(),
            in_universe: false,
            retained: false,
            updated_at: self.updated_at.unwrap_or(fetched_at),
        })
    }
}

#[derive(Debug, Deserialize)]
struct SnapshotDto {
    symbol: String,
    price: Option<f64>,
    #[serde(default)]
    market_cap: Option<f64>,
    previous_close: Option<f64>,
    open: Option<f64>,
    high: Option<f64>,
    low: Option<f64>,
    volume: Option<f64>,
    as_of: DateTime<Utc>,
}

impl SnapshotDto {
    fn into_domain(self, requested: &HashSet<String>) -> Result<Snapshot, ProviderError> {
        let symbol = normalize_symbol(&self.symbol, "snapshot")?;
        if !requested.contains(&symbol)
            || !valid_optional_positive(self.price)
            || !valid_optional_positive(self.market_cap)
            || !valid_optional_positive(self.previous_close)
            || !valid_optional_positive(self.open)
            || !valid_optional_positive(self.high)
            || !valid_optional_positive(self.low)
            || !valid_optional_non_negative(self.volume)
            || self
                .high
                .zip(self.low)
                .is_some_and(|(high, low)| high < low)
        {
            return Err(ProviderError::InvalidData {
                resource: "snapshot",
            });
        }
        Ok(Snapshot {
            symbol,
            price: self.price,
            market_cap: self.market_cap,
            previous_close: self.previous_close,
            open: self.open,
            high: self.high,
            low: self.low,
            volume: self.volume,
            updated_at: self.as_of,
        })
    }
}

#[derive(Debug, Deserialize)]
struct BarDto {
    symbol: String,
    timeframe: String,
    timestamp: DateTime<Utc>,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    trade_count: Option<u64>,
    vwap: Option<f64>,
    source: String,
}

impl BarDto {
    fn into_domain(
        self,
        requested_timeframe: &str,
        requested_symbols: &HashSet<String>,
    ) -> Result<Bar, ProviderError> {
        let symbol = normalize_symbol(&self.symbol, "bar")?;
        if !requested_symbols.contains(&symbol)
            || self.timeframe != requested_timeframe
            || !valid_positive(self.open)
            || !valid_positive(self.high)
            || !valid_positive(self.low)
            || !valid_positive(self.close)
            || !valid_non_negative(self.volume)
            || !valid_optional_positive(self.vwap)
            || self.high < self.low
            || self.open > self.high
            || self.open < self.low
            || self.close > self.high
            || self.close < self.low
        {
            return Err(ProviderError::InvalidData { resource: "bar" });
        }
        Ok(Bar {
            symbol,
            timeframe: self.timeframe,
            timestamp: self.timestamp,
            open: self.open,
            high: self.high,
            low: self.low,
            close: self.close,
            volume: self.volume,
            trade_count: self.trade_count,
            vwap: self.vwap,
            source: validate_text(self.source, MAX_NAME_LENGTH, "bar")?,
        })
    }
}

#[derive(Debug, Deserialize)]
struct NewsDto {
    id: String,
    headline: String,
    source: String,
    published_at: DateTime<Utc>,
    url: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    symbols: Vec<String>,
}

impl NewsDto {
    fn into_domain(self) -> Result<NewsItem, ProviderError> {
        let id = validate_text(self.id, MAX_NAME_LENGTH, "news")?;
        let headline = validate_text(self.headline, MAX_HEADLINE_LENGTH, "news")?;
        let source = validate_text(self.source, MAX_NAME_LENGTH, "news")?;
        let summary = validate_optional_text(self.summary, MAX_SUMMARY_LENGTH, "news")?;
        let url = validate_article_url(&self.url)?;
        Ok(NewsItem {
            id,
            headline,
            source,
            published_at: self.published_at,
            url,
            summary,
            symbols: normalize_symbols(&self.symbols)?,
        })
    }
}

fn validate_base_url(value: &str) -> Result<Url, ProviderError> {
    let normalized = format!("{}/", value.trim().trim_end_matches('/'));
    let url = Url::parse(&normalized).map_err(|_| {
        ProviderError::InvalidRequest("stock API base URL is not a valid URL".to_owned())
    })?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err(ProviderError::InvalidRequest(
            "stock API base URL must use HTTPS (HTTP is allowed only for loopback fixtures)"
                .to_owned(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ProviderError::InvalidRequest(
            "stock API base URL must not contain user information".to_owned(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(ProviderError::InvalidRequest(
            "stock API base URL must not contain a query or fragment".to_owned(),
        ));
    }
    Ok(url)
}

async fn read_bounded_body(
    mut response: Response,
    resource: &'static str,
) -> Result<Vec<u8>, ProviderError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(ProviderError::InvalidData { resource });
    }
    let mut body = Vec::with_capacity(
        response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or_default()
            .min(MAX_RESPONSE_BYTES),
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| ProviderError::Transport {
            kind: transport_error_kind(&error),
        })?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(ProviderError::InvalidData { resource });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn response_error(response: Response) -> ProviderError {
    let status = response.status();
    let message = read_bounded_body(response, "error").await.ok().map_or_else(
        || "request rejected".to_owned(),
        |body| extract_error_message(&body),
    );
    match status {
        StatusCode::UNAUTHORIZED => ProviderError::Authentication,
        StatusCode::FORBIDDEN => ProviderError::Permission {
            status: status.as_u16(),
            message,
        },
        StatusCode::TOO_MANY_REQUESTS => ProviderError::RateLimited { message },
        _ => ProviderError::Api {
            status: status.as_u16(),
            message,
        },
    }
}

fn extract_error_message(body: &[u8]) -> String {
    let message = serde_json::from_slice::<ErrorEnvelope>(body).map_or_else(
        |_| String::from_utf8_lossy(body).into_owned(),
        |envelope| {
            if envelope.error.message.trim().is_empty() {
                envelope.error.code
            } else {
                envelope.error.message
            }
        },
    );
    bounded_text(&message, MAX_ERROR_LENGTH)
}

fn bounded_text(value: &str, maximum: usize) -> String {
    let mut value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut end = value.len().min(maximum);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    if value.is_empty() {
        "request rejected".to_owned()
    } else {
        value
    }
}

fn validate_text(
    value: String,
    maximum: usize,
    resource: &'static str,
) -> Result<String, ProviderError> {
    let value = value.trim();
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(ProviderError::InvalidData { resource });
    }
    Ok(value.to_owned())
}

fn validate_optional_text(
    value: String,
    maximum: usize,
    resource: &'static str,
) -> Result<String, ProviderError> {
    let value = value.trim();
    if value.len() > maximum || value.chars().any(char::is_control) {
        return Err(ProviderError::InvalidData { resource });
    }
    Ok(value.to_owned())
}

fn validate_article_url(value: &str) -> Result<String, ProviderError> {
    let url = Url::parse(value).map_err(|_| ProviderError::InvalidData { resource: "news" })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(ProviderError::InvalidData { resource: "news" });
    }
    Ok(url.to_string())
}

fn normalize_symbols(symbols: &[String]) -> Result<Vec<String>, ProviderError> {
    let mut seen = HashSet::new();
    symbols
        .iter()
        .map(|symbol| normalize_symbol(symbol, "symbol"))
        .filter_map(|result| match result {
            Ok(symbol) if seen.insert(symbol.clone()) => Some(Ok(symbol)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

fn normalize_symbol(value: &str, resource: &'static str) -> Result<String, ProviderError> {
    let symbol = value.trim().to_ascii_uppercase();
    if symbol.is_empty()
        || symbol.len() > 32
        || !symbol
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err(ProviderError::InvalidData { resource });
    }
    Ok(symbol)
}

fn valid_positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn valid_non_negative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

fn valid_optional_positive(value: Option<f64>) -> bool {
    value.is_none_or(valid_positive)
}

fn valid_optional_non_negative(value: Option<f64>) -> bool {
    value.is_none_or(valid_non_negative)
}

fn retryable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

fn parse_retry_after(value: &str) -> Option<Duration> {
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let retry_at = DateTime::parse_from_rfc2822(value)
        .ok()?
        .with_timezone(&Utc);
    retry_at.signed_duration_since(Utc::now()).to_std().ok()
}

fn transport_error_kind(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connection"
    } else if error.is_decode() {
        "response decoding"
    } else {
        "transport"
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };

    use pretty_assertions::assert_eq;

    use super::*;

    #[derive(Debug)]
    struct FixtureResponse {
        status: u16,
        body: String,
        content_length: Option<usize>,
    }

    fn fixture_server(
        responses: Vec<FixtureResponse>,
    ) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let handle = thread::spawn(move || {
            let mut responses = VecDeque::from(responses);
            while let Some(response) = responses.pop_front() {
                let (mut stream, _) = listener.accept().expect("accept fixture request");
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("fixture timeout");
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                loop {
                    let read = stream.read(&mut buffer).expect("read fixture request");
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                captured
                    .lock()
                    .expect("capture request")
                    .push(String::from_utf8_lossy(&request).into_owned());
                let reason = match response.status {
                    200 => "OK",
                    400 => "Bad Request",
                    401 => "Unauthorized",
                    403 => "Forbidden",
                    429 => "Too Many Requests",
                    500 => "Internal Server Error",
                    _ => "Fixture",
                };
                let length = response.content_length.unwrap_or(response.body.len());
                write!(
                    stream,
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.status, reason, length, response.body
                )
                .expect("write fixture response");
            }
        });
        (format!("http://{address}"), requests, handle)
    }

    fn response(body: &str) -> FixtureResponse {
        FixtureResponse {
            status: 200,
            body: body.to_owned(),
            content_length: None,
        }
    }

    fn provider(base_url: &str, news_enabled: bool) -> StockApiProvider {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .expect("fixture client");
        let mut provider =
            StockApiProvider::with_client(base_url, news_enabled, client).expect("provider");
        provider.retry.base_delay = Duration::ZERO;
        provider.retry.max_delay = Duration::ZERO;
        provider
    }

    #[tokio::test]
    async fn maps_the_versioned_contract_without_sending_credentials() {
        let (base_url, requests, server) = fixture_server(vec![
            response(
                r#"{"schema_version":1,"data":[{"symbol":"aapl","name":"Apple Inc.","exchange":"NASDAQ","market_cap":3200000000000.0,"updated_at":"2026-07-28T15:59:00Z"}],"next_page_token":null}"#,
            ),
            response(
                r#"{"schema_version":1,"data":[{"symbol":"AAPL","price":213.5,"market_cap":3210000000000.0,"previous_close":210.0,"open":211.0,"high":214.0,"low":209.5,"volume":12345.0,"as_of":"2026-07-28T16:00:00Z"}],"next_page_token":null}"#,
            ),
            response(
                r#"{"schema_version":1,"data":[{"symbol":"AAPL","timeframe":"1Day","timestamp":"2026-07-27T20:00:00Z","open":210.0,"high":214.0,"low":209.0,"close":213.0,"volume":10000.0,"trade_count":42,"vwap":212.5,"source":"licensed-feed"}],"next_page_token":"page-2"}"#,
            ),
            response(
                r#"{"schema_version":1,"data":[{"symbol":"AAPL","timeframe":"1Day","timestamp":"2026-07-28T20:00:00Z","open":213.0,"high":215.0,"low":212.0,"close":214.0,"volume":11000.0,"trade_count":null,"vwap":null,"source":"licensed-feed"}],"next_page_token":null}"#,
            ),
            response(
                r#"{"schema_version":1,"data":[{"id":"news-1","headline":"Results published","source":"Publisher","published_at":"2026-07-28T18:00:00Z","url":"https://example.test/story","summary":"Quarterly results.","symbols":["aapl"]}],"next_page_token":null}"#,
            ),
        ]);
        let provider = provider(&base_url, true);

        let assets = provider.fetch_assets().await.expect("assets");
        let snapshots = provider
            .fetch_snapshots(&["aapl".to_owned()])
            .await
            .expect("snapshots");
        let bars = provider
            .fetch_bars(
                &["aapl".to_owned()],
                "1Day",
                "2026-07-27T00:00:00Z".parse().expect("start"),
                "2026-07-29T00:00:00Z".parse().expect("end"),
            )
            .await
            .expect("bars");
        let news = provider
            .fetch_news(&["aapl".to_owned()], 20)
            .await
            .expect("news");
        server.join().expect("fixture server");

        assert_eq!(assets[0].symbol, "AAPL");
        assert_eq!(assets[0].market_cap, Some(3_200_000_000_000.0));
        assert_eq!(
            assets[0].updated_at,
            "2026-07-28T15:59:00Z"
                .parse::<DateTime<Utc>>()
                .expect("asset timestamp")
        );
        assert_eq!(snapshots[0].price, Some(213.5));
        assert_eq!(snapshots[0].market_cap, Some(3_210_000_000_000.0));
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].source, "licensed-feed");
        assert_eq!(news[0].id, "news-1");
        let requests = requests.lock().expect("requests");
        assert_eq!(requests.len(), 5);
        assert!(requests[0].contains("GET /v1/assets?status=active "));
        assert!(requests[1].contains("GET /v1/snapshots?symbols=AAPL "));
        assert!(requests[2].contains("timeframe=1Day"));
        assert!(requests[2].contains("adjustment=all"));
        assert!(requests[3].contains("page_token=page-2"));
        assert!(requests[4].contains("GET /v1/news?symbols=AAPL&limit=20 "));
        for request in requests.iter() {
            let lower = request.to_ascii_lowercase();
            assert!(lower.contains("accept: application/json"));
            assert!(!lower.contains("authorization:"));
            assert!(!lower.contains("apca-api"));
            assert!(!lower.contains("api-key"));
        }
    }

    #[test]
    fn validates_transport_and_optional_news_capability() {
        for invalid in [
            "http://example.com",
            "https://user:pass@example.com",
            "https://example.com/api?token=value",
        ] {
            assert!(matches!(
                StockApiProvider::new(invalid, true),
                Err(ProviderError::InvalidRequest(_))
            ));
        }
        let provider = StockApiProvider::new("https://example.com/api", false)
            .expect("HTTPS provider")
            .into_provider_set();
        assert_eq!(provider.id(), "stock-api");
        assert!(!provider.supports_news());

        let provider =
            StockApiProvider::new("https://example.com/api", false).expect("path base URL");
        assert_eq!(
            provider
                .endpoint("v1/assets")
                .expect("asset endpoint")
                .as_str(),
            "https://example.com/api/v1/assets"
        );
    }

    #[test]
    fn snapshot_market_cap_must_be_positive_and_finite_when_present() {
        let requested = HashSet::from(["AAPL".to_owned()]);
        let snapshot = |market_cap| SnapshotDto {
            symbol: "AAPL".to_owned(),
            price: Some(213.5),
            market_cap,
            previous_close: Some(210.0),
            open: Some(211.0),
            high: Some(214.0),
            low: Some(209.5),
            volume: Some(12_345.0),
            as_of: "2026-07-28T16:00:00Z".parse().expect("timestamp"),
        };

        assert!(snapshot(None).into_domain(&requested).is_ok());
        assert!(
            snapshot(Some(3_200_000_000_000.0))
                .into_domain(&requested)
                .is_ok()
        );
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(matches!(
                snapshot(Some(invalid)).into_domain(&requested),
                Err(ProviderError::InvalidData {
                    resource: "snapshot"
                })
            ));
        }
    }

    #[tokio::test]
    async fn rejects_empty_bar_ranges_before_making_a_request() {
        let provider = StockApiProvider::new("https://example.com/api", false).expect("provider");
        let instant = "2026-07-28T00:00:00Z".parse().expect("instant");

        assert!(matches!(
            provider
                .fetch_bars(&["AAPL".to_owned()], "1Day", instant, instant)
                .await,
            Err(ProviderError::InvalidRequest(_))
        ));
    }

    #[tokio::test]
    async fn rejects_unknown_schema_and_oversized_responses() {
        let (base_url, _, server) = fixture_server(vec![
            response(r#"{"schema_version":2,"data":[],"next_page_token":null}"#),
            FixtureResponse {
                status: 200,
                body: "{}".to_owned(),
                content_length: Some(MAX_RESPONSE_BYTES + 1),
            },
        ]);
        let provider = provider(&base_url, false);

        assert!(matches!(
            provider.fetch_assets().await,
            Err(ProviderError::InvalidData { resource: "asset" })
        ));
        assert!(matches!(
            provider.fetch_assets().await,
            Err(ProviderError::InvalidData { resource: "asset" })
        ));
        server.join().expect("fixture server");
    }

    #[tokio::test]
    async fn maps_safe_error_envelopes_without_adapter_names() {
        let (base_url, _, server) = fixture_server(vec![FixtureResponse {
            status: 403,
            body: r#"{"error":{"code":"not_entitled","message":"Requested data is unavailable"}}"#
                .to_owned(),
            content_length: None,
        }]);
        let provider = provider(&base_url, false);

        let error = provider.fetch_assets().await.expect_err("permission error");
        server.join().expect("fixture server");

        let rendered = error.to_string();
        assert!(rendered.contains("Requested data is unavailable"));
        assert!(!rendered.to_ascii_lowercase().contains("alpaca"));
    }
}
