//! External data-provider boundaries.

pub mod alpaca;
pub mod stock_api;

use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use tokio::sync::Mutex;

use crate::{
    domain::{Bar, Company, NewsItem, Snapshot, StockSplit},
    market::{CacheIdentity, MarketContext},
};

pub use alpaca::AlpacaProvider;
pub use stock_api::StockApiProvider;

const CORPORATE_ACTION_CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Failures shared by all provider adapters.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider credentials are required")]
    MissingCredentials,
    #[error("provider authentication failed")]
    Authentication,
    #[error("the provider account is not entitled to this resource: {message}")]
    Permission { status: u16, message: String },
    #[error("the provider rate limit remained active after bounded retries: {message}")]
    RateLimited { message: String },
    #[error("provider request failed with HTTP {status}: {message}")]
    Api { status: u16, message: String },
    #[error("could not reach the provider after bounded retries ({kind})")]
    Transport { kind: &'static str },
    #[error("the provider returned invalid {resource} data")]
    InvalidData { resource: &'static str },
    #[error("invalid provider request: {0}")]
    InvalidRequest(String),
}

impl ProviderError {
    pub(crate) fn allows_feed_fallback(&self) -> bool {
        matches!(
            self,
            Self::Permission { status: 403, .. } | Self::Api { status: 422, .. }
        )
    }

    pub(crate) fn is_invalid_symbol(&self) -> bool {
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

/// Active, searchable instruments available from a provider.
#[async_trait]
pub trait AssetProvider: Send + Sync {
    async fn fetch_assets(&self) -> Result<Vec<Company>, ProviderError>;
}

/// Prices and historical bars needed by the cache synchronizer.
#[async_trait]
pub trait MarketDataProvider: Send + Sync {
    async fn fetch_bars(
        &self,
        symbols: &[String],
        timeframe: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Bar>, ProviderError>;

    async fn fetch_snapshots(&self, symbols: &[String]) -> Result<Vec<Snapshot>, ProviderError>;

    /// Latest timestamp that can be requested without entering a provider's
    /// delayed-data window.
    fn latest_historical_end(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        now
    }
}

/// Split history used to reconcile dated share counts with current prices.
#[async_trait]
pub trait CorporateActionsProvider: Send + Sync {
    async fn fetch_stock_splits(
        &self,
        symbols: &[String],
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Vec<StockSplit>, ProviderError>;
}

/// News is deliberately separate so callers fetch it only for visible ticker details.
#[async_trait]
pub trait NewsProvider: Send + Sync {
    async fn fetch_news(
        &self,
        symbols: &[String],
        limit: usize,
    ) -> Result<Vec<NewsItem>, ProviderError>;
}

#[derive(Debug)]
struct CachedStockSplits {
    start: NaiveDate,
    end: NaiveDate,
    fetched_at: Instant,
    splits: Vec<StockSplit>,
}

#[derive(Debug, Default)]
struct CorporateActionsCache {
    by_symbol: HashMap<String, CachedStockSplits>,
    retry_after: Option<Instant>,
}

/// Provider-neutral capabilities used by synchronization.
///
/// The capabilities may come from one adapter or from independent adapters.
/// News is optional because a quote provider need not also be a news source.
#[derive(Clone)]
pub struct ProviderSet {
    id: Arc<str>,
    display_name: Arc<str>,
    cache_namespace: Arc<str>,
    market_context: MarketContext,
    assets: Arc<dyn AssetProvider>,
    market_data: Arc<dyn MarketDataProvider>,
    corporate_actions: Option<Arc<dyn CorporateActionsProvider>>,
    corporate_actions_cache: Arc<Mutex<CorporateActionsCache>>,
    news: Option<Arc<dyn NewsProvider>>,
}

impl ProviderSet {
    #[must_use]
    pub fn new(
        id: impl Into<Arc<str>>,
        display_name: impl Into<Arc<str>>,
        assets: Arc<dyn AssetProvider>,
        market_data: Arc<dyn MarketDataProvider>,
    ) -> Self {
        let id = id.into();
        Self {
            cache_namespace: id.clone(),
            id,
            display_name: display_name.into(),
            market_context: MarketContext::default(),
            assets,
            market_data,
            corporate_actions: None,
            corporate_actions_cache: Arc::new(Mutex::new(CorporateActionsCache::default())),
            news: None,
        }
    }

    /// Build a complete capability set from one adapter.
    #[must_use]
    pub fn from_full_provider<P>(
        id: impl Into<Arc<str>>,
        display_name: impl Into<Arc<str>>,
        provider: Arc<P>,
    ) -> Self
    where
        P: AssetProvider + MarketDataProvider + NewsProvider + 'static,
    {
        let assets: Arc<dyn AssetProvider> = provider.clone();
        let market_data: Arc<dyn MarketDataProvider> = provider.clone();
        let news: Arc<dyn NewsProvider> = provider;
        Self::new(id, display_name, assets, market_data).with_news(news)
    }

    #[must_use]
    pub fn with_corporate_actions(
        mut self,
        corporate_actions: Arc<dyn CorporateActionsProvider>,
    ) -> Self {
        self.corporate_actions = Some(corporate_actions);
        self
    }

    #[must_use]
    pub fn with_news(mut self, news: Arc<dyn NewsProvider>) -> Self {
        self.news = Some(news);
        self
    }

    #[must_use]
    pub fn with_cache_namespace(mut self, cache_namespace: impl Into<Arc<str>>) -> Self {
        self.cache_namespace = cache_namespace.into();
        self
    }

    #[must_use]
    pub fn with_market_context(mut self, market_context: MarketContext) -> Self {
        self.market_context = market_context;
        self
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    #[must_use]
    pub fn cache_identity(&self) -> CacheIdentity {
        CacheIdentity::new(self.cache_namespace.clone(), self.market_context.clone())
    }

    #[must_use]
    pub fn market_context(&self) -> &MarketContext {
        &self.market_context
    }

    #[must_use]
    pub fn supports_news(&self) -> bool {
        self.news.is_some()
    }

    #[must_use]
    pub fn supports_corporate_actions(&self) -> bool {
        self.corporate_actions.is_some()
    }

    pub async fn fetch_assets(&self) -> Result<Vec<Company>, ProviderError> {
        self.assets.fetch_assets().await
    }

    pub async fn fetch_bars(
        &self,
        symbols: &[String],
        timeframe: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Bar>, ProviderError> {
        self.market_data
            .fetch_bars(symbols, timeframe, start, end)
            .await
    }

    pub async fn fetch_snapshots(
        &self,
        symbols: &[String],
    ) -> Result<Vec<Snapshot>, ProviderError> {
        self.market_data.fetch_snapshots(symbols).await
    }

    pub async fn fetch_stock_splits(
        &self,
        symbols: &[String],
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Option<Vec<StockSplit>>, ProviderError> {
        if end < start {
            return Err(ProviderError::InvalidRequest(
                "stock-split end must not precede start".to_owned(),
            ));
        }
        let Some(corporate_actions) = &self.corporate_actions else {
            return Ok(None);
        };
        let mut seen = HashSet::new();
        let symbols = symbols
            .iter()
            .map(|symbol| symbol.trim().to_ascii_uppercase())
            .filter(|symbol| !symbol.is_empty() && seen.insert(symbol.clone()))
            .collect::<Vec<_>>();
        if symbols.is_empty() {
            return Ok(Some(Vec::new()));
        }

        let now = Instant::now();
        let (missing, mut result, cooling_down) = {
            let mut cache = self.corporate_actions_cache.lock().await;
            cache.by_symbol.retain(|_, cached| {
                now.saturating_duration_since(cached.fetched_at) < CORPORATE_ACTION_CACHE_TTL
            });
            let mut missing = Vec::new();
            let mut result = Vec::new();
            for symbol in &symbols {
                match cache.by_symbol.get(symbol) {
                    Some(cached) if cached.start <= start && cached.end >= end => {
                        result.extend(
                            cached
                                .splits
                                .iter()
                                .filter(|split| {
                                    split.effective_date >= start && split.effective_date <= end
                                })
                                .cloned(),
                        );
                    }
                    _ => missing.push(symbol.clone()),
                }
            }
            let cooling_down = cache
                .retry_after
                .is_some_and(|retry_after| retry_after > now);
            (missing, result, cooling_down)
        };
        if missing.is_empty() {
            sort_and_deduplicate_splits(&mut result);
            return Ok(Some(result));
        }
        if cooling_down {
            return Err(ProviderError::Api {
                status: 503,
                message: "corporate-action lookup is cooling down after a recent failure"
                    .to_owned(),
            });
        }

        let fetched = match corporate_actions
            .fetch_stock_splits(&missing, start, end)
            .await
        {
            Ok(fetched) => fetched,
            Err(error) => {
                self.corporate_actions_cache.lock().await.retry_after =
                    Some(now + CORPORATE_ACTION_CACHE_TTL);
                return Err(error);
            }
        };
        let missing_set = missing.iter().map(String::as_str).collect::<HashSet<_>>();
        let fetched = fetched
            .into_iter()
            .filter_map(|mut split| {
                split.symbol = split.symbol.trim().to_ascii_uppercase();
                missing_set.contains(split.symbol.as_str()).then_some(split)
            })
            .collect::<Vec<_>>();
        {
            let mut cache = self.corporate_actions_cache.lock().await;
            cache.retry_after = None;
            for symbol in missing {
                let splits = fetched
                    .iter()
                    .filter(|split| split.symbol == symbol)
                    .cloned()
                    .collect::<Vec<_>>();
                result.extend(splits.iter().cloned());
                cache.by_symbol.insert(
                    symbol,
                    CachedStockSplits {
                        start,
                        end,
                        fetched_at: now,
                        splits,
                    },
                );
            }
        }
        sort_and_deduplicate_splits(&mut result);
        Ok(Some(result))
    }

    #[must_use]
    pub fn latest_historical_end(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        self.market_data.latest_historical_end(now)
    }

    pub async fn fetch_news(
        &self,
        symbols: &[String],
        limit: usize,
    ) -> Result<Option<Vec<NewsItem>>, ProviderError> {
        match &self.news {
            Some(news) => news.fetch_news(symbols, limit).await.map(Some),
            None => Ok(None),
        }
    }
}

fn sort_and_deduplicate_splits(splits: &mut Vec<StockSplit>) {
    splits.sort_unstable_by(|left, right| {
        left.symbol
            .cmp(&right.symbol)
            .then(left.effective_date.cmp(&right.effective_date))
    });
    splits.dedup_by(|left, right| left == right);
}

impl fmt::Debug for ProviderSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderSet")
            .field("id", &self.id)
            .field("display_name", &self.display_name)
            .field("cache_namespace", &self.cache_namespace)
            .field("market_context", &self.market_context)
            .field("corporate_actions", &self.supports_corporate_actions())
            .field("news", &self.supports_news())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Debug)]
    struct StubAssets;

    #[async_trait]
    impl AssetProvider for StubAssets {
        async fn fetch_assets(&self) -> Result<Vec<Company>, ProviderError> {
            Ok(Vec::new())
        }
    }

    #[derive(Debug)]
    struct StubMarketData;

    #[async_trait]
    impl MarketDataProvider for StubMarketData {
        async fn fetch_bars(
            &self,
            _symbols: &[String],
            _timeframe: &str,
            _start: DateTime<Utc>,
            _end: DateTime<Utc>,
        ) -> Result<Vec<Bar>, ProviderError> {
            Ok(Vec::new())
        }

        async fn fetch_snapshots(
            &self,
            _symbols: &[String],
        ) -> Result<Vec<Snapshot>, ProviderError> {
            Ok(Vec::new())
        }

        fn latest_historical_end(&self, now: DateTime<Utc>) -> DateTime<Utc> {
            now - chrono::Duration::minutes(15)
        }
    }

    #[derive(Debug)]
    struct StubNews;

    #[async_trait]
    impl NewsProvider for StubNews {
        async fn fetch_news(
            &self,
            _symbols: &[String],
            _limit: usize,
        ) -> Result<Vec<NewsItem>, ProviderError> {
            Ok(Vec::new())
        }
    }

    #[derive(Debug, Default)]
    struct StubCorporateActions {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl CorporateActionsProvider for StubCorporateActions {
        async fn fetch_stock_splits(
            &self,
            symbols: &[String],
            _start: NaiveDate,
            _end: NaiveDate,
        ) -> Result<Vec<StockSplit>, ProviderError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(symbols
                .iter()
                .filter(|symbol| symbol.as_str() == "INHD")
                .map(|symbol| StockSplit {
                    symbol: symbol.clone(),
                    effective_date: NaiveDate::from_ymd_opt(2026, 5, 4).expect("split date"),
                    old_rate: 20.0,
                    new_rate: 1.0,
                })
                .collect())
        }
    }

    #[tokio::test]
    async fn provider_set_routes_independent_capabilities() {
        let providers = ProviderSet::new(
            "fixture",
            "Fixture",
            Arc::new(StubAssets),
            Arc::new(StubMarketData),
        )
        .with_news(Arc::new(StubNews));
        let now = "2026-07-25T12:00:00Z".parse().expect("timestamp");

        assert_eq!(providers.id(), "fixture");
        assert_eq!(providers.display_name(), "Fixture");
        assert_eq!(providers.cache_identity().namespace.as_ref(), "fixture");
        assert_eq!(providers.market_context(), &MarketContext::us_equities());
        assert!(providers.supports_news());
        assert!(providers.fetch_assets().await.expect("assets").is_empty());
        assert!(
            providers
                .fetch_snapshots(&["TEST".to_owned()])
                .await
                .expect("snapshots")
                .is_empty()
        );
        assert_eq!(
            providers.latest_historical_end(now),
            now - chrono::Duration::minutes(15)
        );
        assert!(
            providers
                .fetch_news(&["TEST".to_owned()], 5)
                .await
                .expect("news capability")
                .expect("news response")
                .is_empty()
        );
    }

    #[test]
    fn provider_set_cache_identity_can_describe_an_independent_market() {
        let mut market = MarketContext::us_equities();
        market.id = Arc::from("fixture-market");
        market.symbol_namespace = Arc::from("fixture-symbols");
        let providers = ProviderSet::new(
            "fixture",
            "Fixture",
            Arc::new(StubAssets),
            Arc::new(StubMarketData),
        )
        .with_cache_namespace("fixture:v2|feed=delayed")
        .with_market_context(market.clone());

        assert_eq!(
            providers.cache_identity(),
            CacheIdentity::new("fixture:v2|feed=delayed", market)
        );
    }

    #[tokio::test]
    async fn news_is_an_optional_capability() {
        let providers = ProviderSet::new(
            "fixture",
            "Fixture",
            Arc::new(StubAssets),
            Arc::new(StubMarketData),
        );

        assert!(!providers.supports_news());
        assert!(
            providers
                .fetch_news(&["TEST".to_owned()], 5)
                .await
                .expect("unsupported news is not an error")
                .is_none()
        );
    }

    #[tokio::test]
    async fn corporate_action_cache_reuses_full_refresh_coverage_for_ticker_requests() {
        let corporate_actions = Arc::new(StubCorporateActions::default());
        let providers = ProviderSet::new(
            "fixture",
            "Fixture",
            Arc::new(StubAssets),
            Arc::new(StubMarketData),
        )
        .with_corporate_actions(corporate_actions.clone());
        let start = NaiveDate::from_ymd_opt(2024, 1, 1).expect("start");
        let later_start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("later start");
        let post_split_start = NaiveDate::from_ymd_opt(2026, 6, 1).expect("post-split start");
        let end = NaiveDate::from_ymd_opt(2026, 7, 29).expect("end");

        let initial = providers
            .fetch_stock_splits(&["INHD".to_owned(), "NVDA".to_owned()], start, end)
            .await
            .expect("initial lookup")
            .expect("supported capability");
        let ticker = providers
            .fetch_stock_splits(&["INHD".to_owned()], later_start, end)
            .await
            .expect("cached ticker lookup")
            .expect("supported capability");
        let empty_ticker = providers
            .fetch_stock_splits(&["NVDA".to_owned()], later_start, end)
            .await
            .expect("cached empty ticker lookup")
            .expect("supported capability");
        let post_split = providers
            .fetch_stock_splits(&["INHD".to_owned()], post_split_start, end)
            .await
            .expect("cached narrowed ticker lookup")
            .expect("supported capability");

        assert_eq!(initial.len(), 1);
        assert_eq!(ticker, initial);
        assert!(empty_ticker.is_empty());
        assert!(post_split.is_empty());
        assert_eq!(corporate_actions.calls.load(Ordering::Relaxed), 1);

        providers
            .fetch_stock_splits(&["OTHER".to_owned()], later_start, end)
            .await
            .expect("uncached ticker lookup");
        assert_eq!(corporate_actions.calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn shared_errors_do_not_name_an_adapter() {
        let rendered = ProviderError::Authentication.to_string();
        assert_eq!(rendered, "provider authentication failed");
        assert!(!rendered.to_ascii_lowercase().contains("alpaca"));
    }
}
