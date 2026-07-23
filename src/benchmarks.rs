//! Broad-market benchmarks backed by liquid ETF proxies.

use chrono::{DateTime, Utc};

use crate::domain::Company;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketBenchmark {
    pub label: &'static str,
    pub symbol: &'static str,
    pub fund_name: &'static str,
    pub exchange: &'static str,
}

impl MarketBenchmark {
    pub const ALL: [Self; 3] = [
        Self {
            label: "S&P 500",
            symbol: "SPY",
            fund_name: "SPDR S&P 500 ETF Trust",
            exchange: "NYSEARCA",
        },
        Self {
            label: "DOW",
            symbol: "DIA",
            fund_name: "SPDR Dow Jones Industrial Average ETF Trust",
            exchange: "NYSEARCA",
        },
        Self {
            label: "NASDAQ 100",
            symbol: "QQQ",
            fund_name: "Invesco QQQ Trust",
            exchange: "NASDAQ",
        },
    ];

    #[must_use]
    pub fn for_symbol(symbol: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|benchmark| benchmark.symbol == symbol)
    }

    #[must_use]
    pub fn company(self, now: DateTime<Utc>) -> Company {
        Company {
            symbol: self.symbol.to_owned(),
            name: self.fund_name.to_owned(),
            sector: None,
            raw_sector: Some("Broad-market ETF proxy".to_owned()),
            exchange: self.exchange.to_owned(),
            industry: "Market benchmark ETF".to_owned(),
            market_cap: None,
            shares_outstanding: None,
            rank: None,
            description: format!(
                "{} is used as the liquid {} benchmark proxy. Its price and return are ETF data, not the literal index level.",
                self.symbol, self.label
            ),
            in_universe: true,
            retained: true,
            updated_at: now,
        }
    }
}

#[must_use]
pub fn companies(now: DateTime<Utc>) -> Vec<Company> {
    MarketBenchmark::ALL
        .into_iter()
        .map(|benchmark| benchmark.company(now))
        .collect()
}

#[must_use]
pub fn is_benchmark_symbol(symbol: &str) -> bool {
    MarketBenchmark::for_symbol(symbol).is_some()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn benchmark_companies_are_retained_without_entering_a_sector() {
        let now = Utc.with_ymd_and_hms(2026, 7, 23, 20, 0, 0).unwrap();
        let companies = companies(now);

        assert_eq!(
            companies
                .iter()
                .map(|company| company.symbol.as_str())
                .collect::<Vec<_>>(),
            ["SPY", "DIA", "QQQ"]
        );
        assert!(companies.iter().all(|company| company.sector.is_none()));
        assert!(
            companies
                .iter()
                .all(|company| company.in_universe && company.retained)
        );
    }
}
