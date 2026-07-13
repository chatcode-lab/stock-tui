use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet, VecDeque},
    fmt,
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::{Client, Response, StatusCode, Url, header::HeaderMap};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, de::DeserializeOwned};
use tokio::{sync::Mutex, time::Instant};

use super::{MarketDataProvider, NewsProvider};
use crate::{
    config::Settings,
    domain::{Bar, Company, NewsItem, Snapshot},
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_MAX_RETRIES: usize = 3;
const DEFAULT_RETRY_BASE: Duration = Duration::from_millis(250);
const DEFAULT_MAX_RETRY_DELAY: Duration = Duration::from_secs(30);
const MAX_NEWS_ITEMS: usize = 50;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("Alpaca credentials are required")]
    MissingCredentials,
    #[error("Alpaca credentials are invalid")]
    Authentication,
    #[error("the Alpaca account is not entitled to this resource: {message}")]
    Permission { status: u16, message: String },
    #[error("Alpaca rate limit remained active after bounded retries: {message}")]
    RateLimited { message: String },
    #[error("Alpaca request failed with HTTP {status}: {message}")]
    Api { status: u16, message: String },
    #[error("could not reach Alpaca after bounded retries ({kind})")]
    Transport { kind: &'static str },
    #[error("Alpaca returned invalid {resource} data")]
    InvalidData { resource: &'static str },
    #[error("invalid provider request: {0}")]
    InvalidRequest(String),
}

impl ProviderError {
    fn allows_feed_fallback(&self) -> bool {
        matches!(
            self,
            Self::Permission { status: 403, .. } | Self::Api { status: 422, .. }
        )
    }

    fn is_invalid_symbol(&self) -> bool {
        let Self::Api { status, message } = self else {
            return false;
        };
        if !matches!(status, 400 | 422) {
            return false;
        }
        let message = message.to_ascii_lowercase();
        message.contains("invalid symbol") || message.contains("unknown symbol")
    }
}

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
    fn delay(&self, attempt: usize, headers: Option<&HeaderMap>) -> Duration {
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

#[derive(Debug)]
struct TokenBucketState {
    tokens: f64,
    last_refill: Instant,
}

#[derive(Debug)]
struct RequestLimiter {
    capacity: f64,
    refill_per_second: f64,
    state: Mutex<TokenBucketState>,
}

impl RequestLimiter {
    fn new(requests_per_minute: u32) -> Self {
        let requests_per_minute = requests_per_minute.max(1);
        let capacity = f64::from(requests_per_minute);
        Self {
            capacity,
            refill_per_second: capacity / 60.0,
            state: Mutex::new(TokenBucketState {
                tokens: capacity,
                last_refill: Instant::now(),
            }),
        }
    }

    async fn acquire(&self) {
        let delay = {
            let mut state = self.state.lock().await;
            let now = Instant::now();
            let elapsed = now.saturating_duration_since(state.last_refill);
            state.tokens =
                (state.tokens + elapsed.as_secs_f64() * self.refill_per_second).min(self.capacity);
            state.last_refill = now;
            if state.tokens >= 1.0 {
                state.tokens -= 1.0;
                None
            } else {
                // Reserve this caller's token so concurrent requests receive successively
                // later slots instead of waking together at the edge of a rate window.
                state.tokens -= 1.0;
                Some(Duration::from_secs_f64(
                    -state.tokens / self.refill_per_second,
                ))
            }
        };
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
    }
}

/// Async Alpaca client with bounded retries and normalized domain responses.
pub struct AlpacaProvider {
    client: Client,
    key: SecretString,
    secret: SecretString,
    data_url: String,
    trading_url: String,
    feed: String,
    snapshot_batch_size: usize,
    history_batch_size: usize,
    limiter: Arc<RequestLimiter>,
    retry: RetryPolicy,
}

impl fmt::Debug for AlpacaProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AlpacaProvider")
            .field("credentials", &"[redacted]")
            .field("data_url", &self.data_url)
            .field("trading_url", &self.trading_url)
            .field("feed", &self.feed)
            .field("snapshot_batch_size", &self.snapshot_batch_size)
            .field("history_batch_size", &self.history_batch_size)
            .finish_non_exhaustive()
    }
}

impl AlpacaProvider {
    pub fn new(settings: &Settings) -> Result<Self, ProviderError> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .user_agent(concat!("stock-tui/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| ProviderError::Transport {
                kind: "client setup",
            })?;
        Self::with_client(settings, client)
    }

    /// Construct with an existing client, primarily for controlled transports and proxies.
    pub fn with_client(settings: &Settings, client: Client) -> Result<Self, ProviderError> {
        let credentials = settings
            .credentials
            .as_ref()
            .ok_or(ProviderError::MissingCredentials)?;
        let data_url = validate_base_url(&settings.data_url, "market data")?;
        let trading_url = validate_base_url(&settings.trading_url, "trading")?;
        Ok(Self {
            client,
            key: credentials.key.clone(),
            secret: credentials.secret.clone(),
            data_url,
            trading_url,
            feed: settings.feed.clone(),
            snapshot_batch_size: settings.snapshot_batch_size.max(1),
            history_batch_size: settings.history_batch_size.max(1),
            limiter: Arc::new(RequestLimiter::new(settings.request_limit_per_minute)),
            retry: RetryPolicy::default(),
        })
    }

    async fn get_json<T>(
        &self,
        url: &str,
        query: &[(String, String)],
        resource: &'static str,
    ) -> Result<T, ProviderError>
    where
        T: DeserializeOwned,
    {
        for attempt in 0..=self.retry.max_retries {
            self.limiter.acquire().await;
            let response = self
                .client
                .get(url)
                .header("APCA-API-KEY-ID", self.key.expose_secret())
                .header("APCA-API-SECRET-KEY", self.secret.expose_secret())
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
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|error| ProviderError::Transport {
                        kind: transport_error_kind(&error),
                    })?;
                return serde_json::from_slice(&bytes)
                    .map_err(|_| ProviderError::InvalidData { resource });
            }

            if retryable_status(status) && attempt < self.retry.max_retries {
                let delay = self.retry.delay(attempt, Some(response.headers()));
                drop(response);
                tokio::time::sleep(delay).await;
                continue;
            }
            return Err(self.response_error(response).await);
        }

        Err(ProviderError::Transport {
            kind: "retry exhaustion",
        })
    }

    async fn response_error(&self, response: Response) -> ProviderError {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let message = extract_error_message(&body);
        let message = self.redact(&message);
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

    fn redact(&self, value: &str) -> String {
        let mut safe = value.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut secrets = [self.key.expose_secret(), self.secret.expose_secret()];
        secrets.sort_unstable_by_key(|secret| Reverse(secret.len()));
        for secret in secrets {
            if !secret.is_empty() {
                safe = safe.replace(secret, "[redacted]");
            }
        }
        let mut end = safe.len().min(240);
        while !safe.is_char_boundary(end) {
            end -= 1;
        }
        safe.truncate(end);
        if safe.is_empty() {
            "request rejected".to_owned()
        } else {
            safe
        }
    }

    fn historical_feed(&self) -> &str {
        if self.feed == "delayed_sip" {
            "sip"
        } else {
            &self.feed
        }
    }

    pub(crate) fn latest_historical_end(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        if self.feed == "delayed_sip" {
            now - chrono::Duration::minutes(16)
        } else {
            now
        }
    }

    async fn snapshots_for_batch(
        &self,
        symbols: &[String],
        feed: &str,
    ) -> Result<Vec<Snapshot>, ProviderError> {
        let query = vec![
            ("symbols".to_owned(), symbols.join(",")),
            ("feed".to_owned(), feed.to_owned()),
        ];
        let response: SnapshotsResponse = self
            .get_json(
                &format!("{}/v2/stocks/snapshots", self.data_url),
                &query,
                "snapshot",
            )
            .await?;
        let snapshots = match response {
            SnapshotsResponse::Wrapped { snapshots } | SnapshotsResponse::Direct(snapshots) => {
                snapshots
            }
        };
        let now = Utc::now();
        Ok(snapshots
            .into_iter()
            .map(|(symbol, snapshot)| snapshot.into_domain(symbol, now))
            .collect())
    }

    async fn snapshots_for_batch_with_fallback(
        &self,
        symbols: &[String],
        candidates: &[String],
        resolved_feed: &mut Option<String>,
    ) -> Result<Vec<Snapshot>, ProviderError> {
        if let Some(feed) = resolved_feed.as_deref() {
            return self.snapshots_for_batch(symbols, feed).await;
        }

        let mut last_error = None;
        for candidate in candidates {
            match self.snapshots_for_batch(symbols, candidate).await {
                Ok(snapshots) => {
                    *resolved_feed = Some(candidate.clone());
                    return Ok(snapshots);
                }
                Err(error) if error.allows_feed_fallback() => last_error = Some(error),
                Err(error) => return Err(error),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            ProviderError::InvalidRequest("no snapshot feed is configured".to_owned())
        }))
    }

    async fn bars_for_batch(
        &self,
        symbols: &[String],
        timeframe: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Bar>, ProviderError> {
        let url = format!("{}/v2/stocks/bars", self.data_url);
        let mut query = vec![
            ("symbols".to_owned(), symbols.join(",")),
            ("timeframe".to_owned(), timeframe.to_owned()),
            (
                "start".to_owned(),
                start.to_rfc3339_opts(SecondsFormat::Secs, true),
            ),
            (
                "end".to_owned(),
                end.to_rfc3339_opts(SecondsFormat::Secs, true),
            ),
            ("limit".to_owned(), "10000".to_owned()),
            ("adjustment".to_owned(), "all".to_owned()),
            ("feed".to_owned(), self.historical_feed().to_owned()),
            ("sort".to_owned(), "asc".to_owned()),
        ];
        let mut result = Vec::new();
        let mut seen_tokens = HashSet::new();
        loop {
            let page: BarsResponse = self.get_json(&url, &query, "bar").await?;
            for (symbol, bars) in page.bars {
                result.extend(
                    bars.into_iter()
                        .map(|bar| bar.into_domain(symbol.clone(), timeframe)),
                );
            }
            let Some(token) = page.next_page_token.filter(|token| !token.is_empty()) else {
                break;
            };
            if !seen_tokens.insert(token.clone()) {
                return Err(ProviderError::InvalidData { resource: "bar" });
            }
            query.retain(|(key, _)| key != "page_token");
            query.push(("page_token".to_owned(), token));
        }
        Ok(result)
    }
}

#[async_trait]
impl MarketDataProvider for AlpacaProvider {
    async fn fetch_assets(&self) -> Result<Vec<Company>, ProviderError> {
        let query = vec![
            ("status".to_owned(), "active".to_owned()),
            ("asset_class".to_owned(), "us_equity".to_owned()),
        ];
        let assets: Vec<AssetDto> = self
            .get_json(&format!("{}/v2/assets", self.trading_url), &query, "asset")
            .await?;
        let updated_at = Utc::now();
        let mut companies = assets
            .into_iter()
            .filter(|asset| !asset.symbol.trim().is_empty())
            .map(|asset| asset.into_domain(updated_at))
            .collect::<Vec<_>>();
        companies.sort_unstable_by(|left, right| left.symbol.cmp(&right.symbol));
        Ok(companies)
    }

    async fn fetch_bars(
        &self,
        symbols: &[String],
        timeframe: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Bar>, ProviderError> {
        if timeframe.trim().is_empty() {
            return Err(ProviderError::InvalidRequest(
                "timeframe must not be empty".to_owned(),
            ));
        }
        if end < start {
            return Err(ProviderError::InvalidRequest(
                "bar end must not precede start".to_owned(),
            ));
        }
        let symbols = normalize_symbols(symbols);
        if symbols.is_empty() {
            return Ok(Vec::new());
        }

        let mut result = Vec::new();
        let mut pending = symbols
            .chunks(self.history_batch_size)
            .map(<[String]>::to_vec)
            .collect::<VecDeque<_>>();
        while let Some(batch) = pending.pop_front() {
            match self.bars_for_batch(&batch, timeframe, start, end).await {
                Ok(bars) => result.extend(bars),
                Err(error) if error.is_invalid_symbol() && batch.len() > 1 => {
                    let midpoint = batch.len() / 2;
                    pending.push_front(batch[midpoint..].to_vec());
                    pending.push_front(batch[..midpoint].to_vec());
                }
                Err(error) if error.is_invalid_symbol() => {
                    tracing::warn!(symbol = %batch[0], "Alpaca does not recognize symbol; skipping bars");
                }
                Err(error) => return Err(error),
            }
        }
        result.sort_unstable_by(|left, right| {
            left.symbol
                .cmp(&right.symbol)
                .then(left.timestamp.cmp(&right.timestamp))
        });
        Ok(result)
    }

    async fn fetch_snapshots(&self, symbols: &[String]) -> Result<Vec<Snapshot>, ProviderError> {
        let symbols = normalize_symbols(symbols);
        if symbols.is_empty() {
            return Ok(Vec::new());
        }
        let candidates = snapshot_feed_candidates(&self.feed);
        let mut resolved_feed: Option<String> = None;
        let mut result = Vec::new();
        let mut pending = symbols
            .chunks(self.snapshot_batch_size)
            .map(<[String]>::to_vec)
            .collect::<VecDeque<_>>();

        while let Some(batch) = pending.pop_front() {
            match self
                .snapshots_for_batch_with_fallback(&batch, &candidates, &mut resolved_feed)
                .await
            {
                Ok(snapshots) => result.extend(snapshots),
                Err(error) if error.is_invalid_symbol() && batch.len() > 1 => {
                    let midpoint = batch.len() / 2;
                    pending.push_front(batch[midpoint..].to_vec());
                    pending.push_front(batch[..midpoint].to_vec());
                }
                Err(error) if error.is_invalid_symbol() => {
                    tracing::warn!(symbol = %batch[0], "Alpaca does not recognize symbol; skipping snapshot");
                }
                Err(error) => return Err(error),
            }
        }

        result.sort_unstable_by(|left, right| left.symbol.cmp(&right.symbol));
        Ok(result)
    }
}

#[async_trait]
impl NewsProvider for AlpacaProvider {
    async fn fetch_news(
        &self,
        symbols: &[String],
        limit: usize,
    ) -> Result<Vec<NewsItem>, ProviderError> {
        let symbols = normalize_symbols(symbols);
        if symbols.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(MAX_NEWS_ITEMS);
        let query = vec![
            ("symbols".to_owned(), symbols.join(",")),
            ("sort".to_owned(), "desc".to_owned()),
            ("limit".to_owned(), limit.to_string()),
            ("include_content".to_owned(), "false".to_owned()),
        ];
        let response: NewsResponse = self
            .get_json(&format!("{}/v1beta1/news", self.data_url), &query, "news")
            .await?;
        let mut items = response
            .news
            .into_iter()
            .map(NewsDto::into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        items.sort_unstable_by_key(|item| Reverse(item.published_at));
        items.truncate(limit);
        Ok(items)
    }
}

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    #[serde(default)]
    message: String,
    #[serde(default)]
    error: String,
}

#[derive(Debug, Deserialize)]
struct AssetDto {
    symbol: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    exchange: String,
}

impl AssetDto {
    fn into_domain(self, updated_at: DateTime<Utc>) -> Company {
        let symbol = self.symbol.trim().to_ascii_uppercase();
        Company {
            name: if self.name.trim().is_empty() {
                symbol.clone()
            } else {
                self.name.trim().to_owned()
            },
            symbol,
            sector: None,
            raw_sector: None,
            exchange: self.exchange,
            industry: String::new(),
            market_cap: None,
            shares_outstanding: None,
            rank: None,
            description: String::new(),
            in_universe: false,
            retained: false,
            updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
struct BarsResponse {
    #[serde(default)]
    bars: HashMap<String, Vec<BarDto>>,
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BarDto {
    #[serde(rename = "t")]
    timestamp: DateTime<Utc>,
    #[serde(rename = "o")]
    open: f64,
    #[serde(rename = "h")]
    high: f64,
    #[serde(rename = "l")]
    low: f64,
    #[serde(rename = "c")]
    close: f64,
    #[serde(rename = "v")]
    volume: f64,
    #[serde(rename = "n")]
    trade_count: Option<u64>,
    #[serde(rename = "vw")]
    vwap: Option<f64>,
}

impl BarDto {
    fn into_domain(self, symbol: String, timeframe: &str) -> Bar {
        Bar {
            symbol: symbol.to_ascii_uppercase(),
            timeframe: timeframe.to_owned(),
            timestamp: self.timestamp,
            open: self.open,
            high: self.high,
            low: self.low,
            close: self.close,
            volume: self.volume,
            trade_count: self.trade_count,
            vwap: self.vwap,
            source: "alpaca".to_owned(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SnapshotsResponse {
    Wrapped {
        snapshots: HashMap<String, SnapshotDto>,
    },
    Direct(HashMap<String, SnapshotDto>),
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotDto {
    #[serde(default)]
    latest_trade: Option<TradeDto>,
    #[serde(default)]
    minute_bar: Option<SnapshotBarDto>,
    #[serde(default)]
    daily_bar: Option<SnapshotBarDto>,
    #[serde(default)]
    prev_daily_bar: Option<SnapshotBarDto>,
}

impl SnapshotDto {
    fn into_domain(self, symbol: String, fallback_time: DateTime<Utc>) -> Snapshot {
        let price = self
            .latest_trade
            .as_ref()
            .and_then(|trade| trade.price)
            .or_else(|| self.minute_bar.as_ref().and_then(|bar| bar.close))
            .or_else(|| self.daily_bar.as_ref().and_then(|bar| bar.close));
        let updated_at = self
            .latest_trade
            .as_ref()
            .and_then(|trade| trade.timestamp)
            .or_else(|| self.minute_bar.as_ref().and_then(|bar| bar.timestamp))
            .or_else(|| self.daily_bar.as_ref().and_then(|bar| bar.timestamp))
            .unwrap_or(fallback_time);
        Snapshot {
            symbol: symbol.to_ascii_uppercase(),
            price,
            previous_close: self.prev_daily_bar.as_ref().and_then(|bar| bar.close),
            open: self.daily_bar.as_ref().and_then(|bar| bar.open),
            high: self.daily_bar.as_ref().and_then(|bar| bar.high),
            low: self.daily_bar.as_ref().and_then(|bar| bar.low),
            volume: self.daily_bar.as_ref().and_then(|bar| bar.volume),
            updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
struct TradeDto {
    #[serde(rename = "p")]
    price: Option<f64>,
    #[serde(rename = "t")]
    timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct SnapshotBarDto {
    #[serde(rename = "t")]
    timestamp: Option<DateTime<Utc>>,
    #[serde(rename = "o")]
    open: Option<f64>,
    #[serde(rename = "h")]
    high: Option<f64>,
    #[serde(rename = "l")]
    low: Option<f64>,
    #[serde(rename = "c")]
    close: Option<f64>,
    #[serde(rename = "v")]
    volume: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct NewsResponse {
    #[serde(default)]
    news: Vec<NewsDto>,
}

#[derive(Debug, Deserialize)]
struct NewsDto {
    id: NewsId,
    headline: String,
    #[serde(default)]
    source: String,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    symbols: Vec<String>,
}

impl NewsDto {
    fn into_domain(self) -> Result<NewsItem, ProviderError> {
        let published_at = self
            .created_at
            .or(self.updated_at)
            .ok_or(ProviderError::InvalidData { resource: "news" })?;
        Ok(NewsItem {
            id: self.id.to_string(),
            headline: self.headline,
            source: if self.source.is_empty() {
                "Alpaca".to_owned()
            } else {
                self.source
            },
            published_at,
            url: self.url,
            summary: self.summary,
            symbols: normalize_symbols(&self.symbols),
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum NewsId {
    String(String),
    Unsigned(u64),
    Signed(i64),
}

impl fmt::Display for NewsId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(value) => formatter.write_str(value),
            Self::Unsigned(value) => write!(formatter, "{value}"),
            Self::Signed(value) => write!(formatter, "{value}"),
        }
    }
}

fn normalize_symbols(symbols: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    symbols
        .iter()
        .map(|symbol| symbol.trim().to_ascii_uppercase())
        .filter(|symbol| !symbol.is_empty() && seen.insert(symbol.clone()))
        .collect()
}

fn validate_base_url(value: &str, label: &str) -> Result<String, ProviderError> {
    let normalized = value.trim_end_matches('/');
    let url = Url::parse(normalized).map_err(|_| {
        ProviderError::InvalidRequest(format!("{label} base URL is not a valid URL"))
    })?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err(ProviderError::InvalidRequest(format!(
            "{label} base URL must use HTTPS (HTTP is allowed only for loopback fixtures)"
        )));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ProviderError::InvalidRequest(format!(
            "{label} base URL must not contain user information"
        )));
    }
    Ok(normalized.to_owned())
}

fn snapshot_feed_candidates(preferred: &str) -> Vec<String> {
    let mut candidates = vec![preferred.to_owned()];
    if preferred == "sip" {
        candidates.push("delayed_sip".to_owned());
    }
    if preferred != "iex" {
        candidates.push("iex".to_owned());
    }
    candidates.dedup();
    candidates
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

fn extract_error_message(body: &str) -> String {
    serde_json::from_str::<ErrorEnvelope>(body).map_or_else(
        |_| body.to_owned(),
        |error| {
            if error.message.is_empty() {
                error.error
            } else {
                error.message
            }
        },
    )
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
        sync::{Arc, Mutex as StdMutex},
        thread,
    };

    use pretty_assertions::assert_eq;
    use secrecy::SecretString;
    use tempfile::TempDir;

    use super::*;
    use crate::config::Credentials;

    #[derive(Debug)]
    struct FixtureResponse {
        status: u16,
        headers: Vec<(&'static str, &'static str)>,
        body: &'static str,
    }

    fn fixture_server(
        responses: Vec<FixtureResponse>,
    ) -> (String, Arc<StdMutex<Vec<String>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("read fixture address");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let handle = thread::spawn(move || {
            let mut responses = VecDeque::from(responses);
            while let Some(response) = responses.pop_front() {
                let (mut stream, _) = listener.accept().expect("accept fixture request");
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("set fixture timeout");
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
                    .expect("record fixture request")
                    .push(String::from_utf8_lossy(&request).into_owned());
                let reason = match response.status {
                    200 => "OK",
                    400 => "Bad Request",
                    403 => "Forbidden",
                    422 => "Unprocessable Entity",
                    429 => "Too Many Requests",
                    500 => "Internal Server Error",
                    _ => "Fixture",
                };
                let mut headers = String::new();
                for (name, value) in response.headers {
                    headers.push_str(&format!("{name}: {value}\r\n"));
                }
                write!(
                    stream,
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n{}",
                    response.status,
                    reason,
                    response.body.len(),
                    headers,
                    response.body
                )
                .expect("write fixture response");
            }
        });
        (format!("http://{address}"), requests, handle)
    }

    fn settings(temp: &TempDir, base_url: &str) -> Settings {
        Settings {
            credentials: Some(Credentials {
                key: SecretString::from("fixture-key".to_owned()),
                secret: SecretString::from("fixture-secret".to_owned()),
            }),
            db_path: temp.path().join("market.sqlite3"),
            config_dir: temp.path().join("config"),
            cache_dir: temp.path().join("cache"),
            data_url: base_url.to_owned(),
            trading_url: base_url.to_owned(),
            feed: "iex".to_owned(),
            refresh_interval: Duration::from_secs(300),
            request_limit_per_minute: 180,
            snapshot_batch_size: 100,
            history_batch_size: 50,
            demo: false,
            offline: false,
            reset_demo: false,
        }
    }

    fn zero_retry_delays(provider: &mut AlpacaProvider) {
        provider.retry.base_delay = Duration::ZERO;
        provider.retry.max_delay = Duration::ZERO;
    }

    #[tokio::test]
    async fn paginated_bars_are_authenticated_and_normalized() {
        let (base_url, requests, server) = fixture_server(vec![
            FixtureResponse {
                status: 200,
                headers: vec![],
                body: r#"{"bars":{"AAPL":[{"t":"2026-07-10T14:30:00-04:00","o":100.0,"h":104.0,"l":99.0,"c":103.0,"v":1000,"n":20,"vw":102.5}]},"next_page_token":"next"}"#,
            },
            FixtureResponse {
                status: 200,
                headers: vec![],
                body: r#"{"bars":{"MSFT":[{"t":"2026-07-10T18:30:00Z","o":200.0,"h":204.0,"l":198.0,"c":203.0,"v":2000}]},"next_page_token":null}"#,
            },
        ]);
        let temp = TempDir::new().expect("temp dir");
        let provider = AlpacaProvider::new(&settings(&temp, &base_url)).expect("provider");
        let bars = provider
            .fetch_bars(
                &["aapl".to_owned(), "MSFT".to_owned(), "AAPL".to_owned()],
                "5Min",
                "2026-07-10T00:00:00Z".parse().expect("start"),
                "2026-07-11T00:00:00Z".parse().expect("end"),
            )
            .await
            .expect("bars");
        server.join().expect("fixture server");

        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].symbol, "AAPL");
        assert_eq!(bars[0].timestamp.to_rfc3339(), "2026-07-10T18:30:00+00:00");
        assert_eq!(bars[0].trade_count, Some(20));
        assert_eq!(bars[0].vwap, Some(102.5));
        let requests = requests.lock().expect("fixture requests");
        assert_eq!(requests.len(), 2);
        assert!(requests[0].contains("apca-api-key-id: fixture-key"));
        assert!(requests[0].contains("apca-api-secret-key: fixture-secret"));
        assert!(requests[0].contains("symbols=AAPL%2CMSFT"));
        assert!(requests[1].contains("page_token=next"));
    }

    #[tokio::test]
    async fn snapshots_fall_back_then_reuse_the_working_feed_across_batches() {
        let (base_url, requests, server) = fixture_server(vec![
            FixtureResponse {
                status: 422,
                headers: vec![],
                body: r#"{"message":"SIP is not permitted"}"#,
            },
            FixtureResponse {
                status: 403,
                headers: vec![],
                body: r#"{"message":"delayed SIP is not permitted"}"#,
            },
            FixtureResponse {
                status: 200,
                headers: vec![],
                body: r#"{"AAPL":{"latestTrade":{"p":105.0,"t":"2026-07-10T15:00:00Z"},"dailyBar":{"t":"2026-07-10T04:00:00Z","o":100.0,"h":106.0,"l":99.0,"c":105.0,"v":1200},"prevDailyBar":{"c":102.0}}}"#,
            },
            FixtureResponse {
                status: 200,
                headers: vec![],
                body: r#"{"MSFT":{"latestTrade":{"p":205.0,"t":"2026-07-10T15:00:00Z"},"dailyBar":{"o":200.0,"h":206.0,"l":199.0,"c":205.0,"v":2200},"prevDailyBar":{"c":202.0}}}"#,
            },
        ]);
        let temp = TempDir::new().expect("temp dir");
        let mut configured = settings(&temp, &base_url);
        configured.feed = "sip".to_owned();
        configured.snapshot_batch_size = 1;
        let provider = AlpacaProvider::new(&configured).expect("provider");
        let snapshots = provider
            .fetch_snapshots(&["AAPL".to_owned(), "MSFT".to_owned()])
            .await
            .expect("snapshots");
        server.join().expect("fixture server");

        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].price, Some(105.0));
        assert_eq!(snapshots[0].previous_close, Some(102.0));
        let requests = requests.lock().expect("fixture requests");
        assert!(requests[0].contains("feed=sip"));
        assert!(requests[1].contains("feed=delayed_sip"));
        assert!(requests[2].contains("feed=iex"));
        assert!(requests[3].contains("feed=iex"));
    }

    #[tokio::test]
    async fn invalid_symbols_are_isolated_without_losing_valid_snapshots() {
        let (base_url, requests, server) = fixture_server(vec![
            FixtureResponse {
                status: 400,
                headers: vec![],
                body: r#"{"message":"invalid symbol: UNKNOWN"}"#,
            },
            FixtureResponse {
                status: 200,
                headers: vec![],
                body: r#"{"AAPL":{"latestTrade":{"p":105.0,"t":"2026-07-10T15:00:00Z"}}}"#,
            },
            FixtureResponse {
                status: 400,
                headers: vec![],
                body: r#"{"message":"invalid symbol: UNKNOWN"}"#,
            },
        ]);
        let temp = TempDir::new().expect("temp dir");
        let provider = AlpacaProvider::new(&settings(&temp, &base_url)).expect("provider");
        let snapshots = provider
            .fetch_snapshots(&["AAPL".to_owned(), "UNKNOWN".to_owned()])
            .await
            .expect("valid snapshots survive invalid symbol");
        server.join().expect("fixture server");

        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].symbol, "AAPL");
        assert_eq!(snapshots[0].price, Some(105.0));
        let requests = requests.lock().expect("fixture requests");
        assert_eq!(requests.len(), 3);
        assert!(requests[0].contains("symbols=AAPL%2CUNKNOWN"));
        assert!(requests[1].contains("symbols=AAPL"));
        assert!(requests[2].contains("symbols=UNKNOWN"));
    }

    #[tokio::test]
    async fn invalid_symbols_are_isolated_without_losing_valid_bars() {
        let (base_url, requests, server) = fixture_server(vec![
            FixtureResponse {
                status: 400,
                headers: vec![],
                body: r#"{"message":"invalid symbol: UNKNOWN"}"#,
            },
            FixtureResponse {
                status: 200,
                headers: vec![],
                body: r#"{"bars":{"AAPL":[{"t":"2026-07-10T14:30:00Z","o":100.0,"h":104.0,"l":99.0,"c":103.0,"v":1000}]},"next_page_token":null}"#,
            },
            FixtureResponse {
                status: 400,
                headers: vec![],
                body: r#"{"message":"invalid symbol: UNKNOWN"}"#,
            },
        ]);
        let temp = TempDir::new().expect("temp dir");
        let provider = AlpacaProvider::new(&settings(&temp, &base_url)).expect("provider");
        let bars = provider
            .fetch_bars(
                &["AAPL".to_owned(), "UNKNOWN".to_owned()],
                "1Day",
                "2026-07-10T00:00:00Z".parse().expect("start"),
                "2026-07-11T00:00:00Z".parse().expect("end"),
            )
            .await
            .expect("valid bars survive invalid symbol");
        server.join().expect("fixture server");

        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].symbol, "AAPL");
        let requests = requests.lock().expect("fixture requests");
        assert_eq!(requests.len(), 3);
        assert!(requests[0].contains("symbols=AAPL%2CUNKNOWN"));
        assert!(requests[1].contains("symbols=AAPL"));
        assert!(requests[2].contains("symbols=UNKNOWN"));
    }

    #[tokio::test]
    async fn retries_rate_limits_and_server_failures_without_leaking_secrets() {
        let (base_url, requests, server) = fixture_server(vec![
            FixtureResponse {
                status: 429,
                headers: vec![("Retry-After", "0")],
                body: r#"{"message":"slow down fixture-secret"}"#,
            },
            FixtureResponse {
                status: 500,
                headers: vec![],
                body: r#"{"message":"temporary"}"#,
            },
            FixtureResponse {
                status: 200,
                headers: vec![],
                body: r#"{"news":[]}"#,
            },
            FixtureResponse {
                status: 400,
                headers: vec![],
                body: r#"{"message":"bad fixture-key and fixture-secret"}"#,
            },
        ]);
        let temp = TempDir::new().expect("temp dir");
        let mut provider = AlpacaProvider::new(&settings(&temp, &base_url)).expect("provider");
        zero_retry_delays(&mut provider);
        assert!(
            provider
                .fetch_news(&["AAPL".to_owned()], 10)
                .await
                .expect("retried news")
                .is_empty()
        );
        let error = provider
            .fetch_news(&["AAPL".to_owned()], 10)
            .await
            .expect_err("API error");
        server.join().expect("fixture server");

        let rendered = error.to_string();
        assert!(!rendered.contains("fixture-key"));
        assert!(!rendered.contains("fixture-secret"));
        assert_eq!(rendered.matches("[redacted]").count(), 2);
        assert_eq!(requests.lock().expect("fixture requests").len(), 4);
    }

    #[tokio::test]
    async fn assets_and_lazy_news_map_to_domain_models() {
        let (base_url, requests, server) = fixture_server(vec![
            FixtureResponse {
                status: 200,
                headers: vec![],
                body: r#"[{"symbol":"MSFT","name":"Microsoft Corporation","exchange":"NASDAQ"},{"symbol":"AAPL","name":"Apple Inc.","exchange":"NASDAQ"}]"#,
            },
            FixtureResponse {
                status: 200,
                headers: vec![],
                body: r#"{"news":[{"id":42,"headline":"Apple publishes results","source":"benzinga","created_at":"2026-07-10T18:00:00Z","updated_at":"2026-07-10T18:01:00Z","url":"https://example.invalid/article","summary":"Results published.","symbols":["aapl"]}]}"#,
            },
        ]);
        let temp = TempDir::new().expect("temp dir");
        let provider = AlpacaProvider::new(&settings(&temp, &base_url)).expect("provider");
        let companies = provider.fetch_assets().await.expect("assets");
        let news = provider
            .fetch_news(&["aapl".to_owned()], 5)
            .await
            .expect("news");
        server.join().expect("fixture server");

        assert_eq!(
            companies
                .iter()
                .map(|company| company.symbol.as_str())
                .collect::<Vec<_>>(),
            vec!["AAPL", "MSFT"]
        );
        assert!(!companies[0].in_universe);
        assert_eq!(news[0].id, "42");
        assert_eq!(news[0].symbols, vec!["AAPL"]);
        let requests = requests.lock().expect("fixture requests");
        assert!(requests[0].contains("GET /v2/assets?"));
        assert!(requests[1].contains("symbols=AAPL"));
        assert!(requests[1].contains("include_content=false"));
    }

    #[test]
    fn provider_debug_and_missing_credentials_are_safe() {
        let temp = TempDir::new().expect("temp dir");
        let mut configured = settings(&temp, "http://127.0.0.1:1");
        let provider = AlpacaProvider::new(&configured).expect("provider");
        let rendered = format!("{provider:?}");
        assert!(!rendered.contains("fixture-key"));
        assert!(!rendered.contains("fixture-secret"));

        configured.credentials = None;
        assert!(matches!(
            AlpacaProvider::new(&configured),
            Err(ProviderError::MissingCredentials)
        ));
    }

    #[test]
    fn provider_urls_cutoffs_and_unicode_redaction_are_bounded() {
        let temp = TempDir::new().expect("temp dir");
        let mut configured = settings(&temp, "http://127.0.0.1:1");
        configured.credentials = Some(Credentials {
            key: SecretString::from("overlap".to_owned()),
            secret: SecretString::from("overlap-secret".to_owned()),
        });
        configured.feed = "delayed_sip".to_owned();
        let provider = AlpacaProvider::new(&configured).expect("provider");
        let rendered = provider.redact(&format!("{} overlap-secret", "é".repeat(200)));
        assert!(rendered.len() <= 240);
        assert!(!rendered.contains("overlap"));

        let now = "2026-07-13T12:00:00Z".parse().expect("timestamp");
        assert_eq!(
            provider.latest_historical_end(now),
            "2026-07-13T11:44:00Z"
                .parse::<DateTime<Utc>>()
                .expect("delayed timestamp")
        );

        configured.data_url = "http://example.com".to_owned();
        assert!(matches!(
            AlpacaProvider::new(&configured),
            Err(ProviderError::InvalidRequest(_))
        ));
    }
}
