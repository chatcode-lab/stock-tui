use std::{fmt, str::FromStr, time::Duration};

use chrono::{DateTime, Days, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sector {
    Consumer,
    Services,
    Healthcare,
    Energy,
    Technology,
    Financial,
    Industrial,
    Materials,
    Utilities,
}

impl Sector {
    pub const ALL: [Self; 9] = [
        Self::Consumer,
        Self::Services,
        Self::Healthcare,
        Self::Energy,
        Self::Technology,
        Self::Financial,
        Self::Industrial,
        Self::Materials,
        Self::Utilities,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Consumer => "Consumer",
            Self::Services => "Services",
            Self::Healthcare => "Healthcare",
            Self::Energy => "Energy",
            Self::Technology => "Technology",
            Self::Financial => "Financial",
            Self::Industrial => "Industrial",
            Self::Materials => "Materials",
            Self::Utilities => "Utilities",
        }
    }

    #[must_use]
    pub fn from_provider(value: &str) -> Option<Self> {
        let normalized = value
            .to_ascii_lowercase()
            .replace('&', "and")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        match normalized.as_str() {
            "consumer"
            | "consumer cyclical"
            | "consumer defensive"
            | "consumer discretionary"
            | "consumer durables"
            | "consumer non-durables"
            | "consumer staples" => Some(Self::Consumer),
            "services" | "communication services" | "miscellaneous" | "telecommunications" => {
                Some(Self::Services)
            }
            "health care" | "healthcare" => Some(Self::Healthcare),
            "energy" => Some(Self::Energy),
            "technology" => Some(Self::Technology),
            "finance" | "financial services" | "financials" | "real estate" => {
                Some(Self::Financial)
            }
            "capital goods" | "industrial" | "industrials" => Some(Self::Industrial),
            "basic industries" | "basic materials" | "materials" => Some(Self::Materials),
            "utilities" => Some(Self::Utilities),
            _ => None,
        }
    }
}

impl fmt::Display for Sector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

impl FromStr for Sector {
    type Err = ParseEnumError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|sector| sector.label().eq_ignore_ascii_case(value))
            .ok_or_else(|| ParseEnumError(value.to_owned()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DateRange {
    #[default]
    #[serde(alias = "Day")]
    Day,
    #[serde(alias = "Week")]
    Week,
    #[serde(alias = "Month")]
    Month,
    #[serde(alias = "ThreeMonths")]
    ThreeMonths,
    #[serde(alias = "SixMonths")]
    SixMonths,
    #[serde(alias = "Year")]
    Year,
    #[serde(alias = "TwoYears")]
    TwoYears,
    #[serde(alias = "FiveYears")]
    FiveYears,
    #[serde(alias = "TenYears")]
    TenYears,
    #[serde(alias = "All")]
    All,
}

impl DateRange {
    pub const ALL: [Self; 10] = [
        Self::Day,
        Self::Week,
        Self::Month,
        Self::ThreeMonths,
        Self::SixMonths,
        Self::Year,
        Self::TwoYears,
        Self::FiveYears,
        Self::TenYears,
        Self::All,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Day => "1D",
            Self::Week => "1W",
            Self::Month => "1M",
            Self::ThreeMonths => "3M",
            Self::SixMonths => "6M",
            Self::Year => "1Y",
            Self::TwoYears => "2Y",
            Self::FiveYears => "5Y",
            Self::TenYears => "10Y",
            Self::All => "ALL",
        }
    }

    #[must_use]
    pub const fn days(self) -> u64 {
        match self {
            Self::Day => 1,
            Self::Week => 7,
            Self::Month => 30,
            Self::ThreeMonths => 91,
            Self::SixMonths => 183,
            Self::Year => 365,
            Self::TwoYears => 731,
            Self::FiveYears => 1_826,
            Self::TenYears => 3_653,
            Self::All => u64::MAX,
        }
    }

    #[must_use]
    pub const fn preferred_timeframe(self) -> &'static str {
        match self {
            Self::Day => "5Min",
            Self::Week | Self::Month => "1Hour",
            Self::FiveYears | Self::TenYears | Self::All => "1Week",
            _ => "1Day",
        }
    }

    #[must_use]
    pub fn cutoff(self, now: DateTime<Utc>) -> DateTime<Utc> {
        if self == Self::All {
            DateTime::UNIX_EPOCH
        } else {
            now.checked_sub_days(Days::new(self.days())).unwrap_or(now)
        }
    }

    #[must_use]
    pub const fn shortcut(self) -> char {
        match self {
            Self::Day => '1',
            Self::Week => '2',
            Self::Month => '3',
            Self::ThreeMonths => '4',
            Self::SixMonths => '5',
            Self::Year => '6',
            Self::TwoYears => '7',
            Self::FiveYears => '8',
            Self::TenYears => '9',
            Self::All => '0',
        }
    }

    #[must_use]
    pub fn previous(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|range| *range == self)
            .unwrap_or(0);
        Self::ALL[index.saturating_sub(1)]
    }

    #[must_use]
    pub fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|range| *range == self)
            .unwrap_or(0);
        Self::ALL[(index + 1).min(Self::ALL.len() - 1)]
    }
}

impl fmt::Display for DateRange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

impl FromStr for DateRange {
    type Err = ParseEnumError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|range| range.label().eq_ignore_ascii_case(value))
            .ok_or_else(|| ParseEnumError(value.to_owned()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SortMode {
    #[default]
    MarketCap,
    Gainers,
    Volume,
    Alphabetical,
}

impl SortMode {
    pub const ALL: [Self; 4] = [
        Self::MarketCap,
        Self::Gainers,
        Self::Volume,
        Self::Alphabetical,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::MarketCap => "Market cap",
            Self::Gainers => "Gainers",
            Self::Volume => "Volume",
            Self::Alphabetical => "A-Z",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Company {
    pub symbol: String,
    pub name: String,
    pub sector: Option<Sector>,
    pub raw_sector: Option<String>,
    pub exchange: String,
    pub industry: String,
    pub market_cap: Option<f64>,
    pub shares_outstanding: Option<f64>,
    pub rank: Option<u16>,
    pub description: String,
    pub in_universe: bool,
    pub retained: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bar {
    pub symbol: String,
    pub timeframe: String,
    pub timestamp: DateTime<Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub trade_count: Option<u64>,
    pub vwap: Option<f64>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub symbol: String,
    pub price: Option<f64>,
    pub previous_close: Option<f64>,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub volume: Option<f64>,
    pub updated_at: DateTime<Utc>,
}

impl Snapshot {
    #[must_use]
    pub fn day_return(&self) -> Option<f64> {
        let price = self.price?;
        let previous = self.previous_close?;
        (previous != 0.0).then_some(price / previous - 1.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsItem {
    pub id: String,
    pub headline: String,
    pub source: String,
    pub published_at: DateTime<Utc>,
    pub url: String,
    pub summary: String,
    pub symbols: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MarketTile {
    pub company: Company,
    pub price: Option<f64>,
    pub period_return: Option<f64>,
    pub volume: Option<f64>,
    pub starred: bool,
    pub stale: bool,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct TickerDetail {
    pub company: Company,
    pub snapshot: Option<Snapshot>,
    pub bars: Vec<Bar>,
    pub news: Vec<NewsItem>,
    pub starred: bool,
    pub period_return: Option<f64>,
    pub sector_return: Option<f64>,
    pub sector_rank: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncPhase {
    Idle,
    Universe,
    Snapshots,
    History,
    News,
    Complete,
    Error,
}

#[derive(Debug, Clone)]
pub struct SyncProgress {
    pub phase: SyncPhase,
    pub completed: usize,
    pub total: usize,
    pub message: String,
    pub last_error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

impl Default for SyncProgress {
    fn default() -> Self {
        Self {
            phase: SyncPhase::Idle,
            completed: 0,
            total: 0,
            message: "Cache ready".to_owned(),
            last_error: None,
            updated_at: Utc::now(),
        }
    }
}

impl SyncProgress {
    #[must_use]
    pub fn fraction(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.completed as f64 / self.total as f64
        }
    }

    #[must_use]
    pub fn stale_after(&self, duration: Duration) -> bool {
        Utc::now()
            .signed_duration_since(self.updated_at)
            .to_std()
            .is_ok_and(|elapsed| elapsed > duration)
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("unknown value {0:?}")]
pub struct ParseEnumError(String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_sector_mapping_is_explicit() {
        assert_eq!(
            Sector::from_provider("Consumer Defensive"),
            Some(Sector::Consumer)
        );
        assert_eq!(
            Sector::from_provider("Real Estate"),
            Some(Sector::Financial)
        );
        assert_eq!(
            Sector::from_provider("Communication Services"),
            Some(Sector::Services)
        );
        assert_eq!(Sector::from_provider("Unknown"), None);
    }

    #[test]
    fn ranges_have_stable_labels_and_order() {
        assert_eq!(
            DateRange::ALL.map(DateRange::label),
            ["1D", "1W", "1M", "3M", "6M", "1Y", "2Y", "5Y", "10Y", "ALL"]
        );
        assert_eq!(DateRange::Month.previous(), DateRange::Week);
        assert_eq!(DateRange::Month.next(), DateRange::ThreeMonths);
        assert_eq!(
            DateRange::ALL.map(DateRange::shortcut),
            ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0']
        );
        assert_eq!(DateRange::All.next(), DateRange::All);
    }

    #[test]
    fn ranges_have_stable_cutoffs_and_serialization() {
        let now = DateTime::parse_from_rfc3339("2026-07-23T12:00:00Z")
            .expect("fixture timestamp")
            .with_timezone(&Utc);
        assert_eq!(
            DateRange::TwoYears.cutoff(now),
            now.checked_sub_days(Days::new(731))
                .expect("fixture cutoff")
        );
        assert_eq!(DateRange::All.cutoff(now), DateTime::UNIX_EPOCH);
        assert_eq!(
            serde_json::to_string(&DateRange::TenYears).expect("range serializes"),
            "\"ten_years\""
        );
        assert_eq!(
            serde_json::from_str::<DateRange>("\"all\"").expect("range deserializes"),
            DateRange::All
        );
        assert_eq!(
            serde_json::from_str::<DateRange>("\"FiveYears\"").expect("legacy range deserializes"),
            DateRange::FiveYears
        );
        assert_eq!(
            "10Y".parse::<DateRange>().expect("label parses"),
            DateRange::TenYears
        );
    }
}
