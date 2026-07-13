//! External data-provider boundaries.

pub mod alpaca;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domain::{Bar, Company, NewsItem, Snapshot};

pub use alpaca::{AlpacaProvider, ProviderError};

/// Market data needed by the cache synchronizer and interactive views.
#[async_trait]
pub trait MarketDataProvider: Send + Sync {
    async fn fetch_assets(&self) -> Result<Vec<Company>, ProviderError>;

    async fn fetch_bars(
        &self,
        symbols: &[String],
        timeframe: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Bar>, ProviderError>;

    async fn fetch_snapshots(&self, symbols: &[String]) -> Result<Vec<Snapshot>, ProviderError>;
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
