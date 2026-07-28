//! External data-provider boundaries.

pub mod alpaca;
pub mod stock_api;

use std::{fmt, sync::Arc};

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domain::{Bar, Company, NewsItem, Snapshot};

pub use alpaca::AlpacaProvider;
pub use stock_api::StockApiProvider;

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

/// News is deliberately separate so callers fetch it only for visible ticker details.
#[async_trait]
pub trait NewsProvider: Send + Sync {
    async fn fetch_news(
        &self,
        symbols: &[String],
        limit: usize,
    ) -> Result<Vec<NewsItem>, ProviderError>;
}

/// Provider-neutral capabilities used by synchronization.
///
/// The capabilities may come from one adapter or from independent adapters.
/// News is optional because a quote provider need not also be a news source.
#[derive(Clone)]
pub struct ProviderSet {
    id: Arc<str>,
    display_name: Arc<str>,
    assets: Arc<dyn AssetProvider>,
    market_data: Arc<dyn MarketDataProvider>,
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
        Self {
            id: id.into(),
            display_name: display_name.into(),
            assets,
            market_data,
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
    pub fn with_news(mut self, news: Arc<dyn NewsProvider>) -> Self {
        self.news = Some(news);
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
    pub fn supports_news(&self) -> bool {
        self.news.is_some()
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

impl fmt::Debug for ProviderSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderSet")
            .field("id", &self.id)
            .field("display_name", &self.display_name)
            .field("news", &self.supports_news())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn shared_errors_do_not_name_an_adapter() {
        let rendered = ProviderError::Authentication.to_string();
        assert_eq!(rendered, "provider authentication failed");
        assert!(!rendered.to_ascii_lowercase().contains("alpaca"));
    }
}
