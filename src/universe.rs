//! Embedded SEC issuer universe.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::domain::{Company, Sector};

const CATALOG_JSON: &str = include_str!("../data/sec_universe.json");
const CATALOG_SCHEMA_VERSION: u32 = 1;
const MIN_COMPANIES_PER_SECTOR: usize = 100;
const MAX_COMPANIES_PER_SECTOR: usize = 250;

#[derive(Debug, Deserialize)]
struct Catalog {
    schema_version: u32,
    companies: Vec<CatalogCompany>,
}

#[derive(Debug, Deserialize)]
struct CatalogCompany {
    rank: u16,
    cik: String,
    symbol: String,
    name: String,
    exchange: String,
    sic: u16,
    sector: Sector,
    public_float: f64,
    shares_outstanding: Option<f64>,
    as_of: String,
    quality: String,
}

/// Loads and validates the build-time SEC catalog as runtime domain companies.
///
/// `EntityPublicFloat` is used only to establish the embedded ranks. It is not
/// copied into `Company::market_cap`; a market cap needs current price data.
pub fn embedded_companies(now: DateTime<Utc>) -> Result<Vec<Company>> {
    let mut catalog: Catalog =
        serde_json::from_str(CATALOG_JSON).context("embedded SEC catalog is invalid JSON")?;
    ensure!(
        catalog.schema_version == CATALOG_SCHEMA_VERSION,
        "unsupported embedded SEC catalog schema {}",
        catalog.schema_version
    );
    for company in &mut catalog.companies {
        company.symbol = normalize_sec_symbol(&company.symbol);
    }
    let mut symbols = HashSet::with_capacity(catalog.companies.len());
    let mut ciks = HashSet::with_capacity(catalog.companies.len());
    let mut ranks: HashMap<Sector, HashSet<u16>> = HashMap::new();
    for company in &catalog.companies {
        ensure!(!company.symbol.trim().is_empty(), "catalog symbol is empty");
        ensure!(
            !company.name.trim().is_empty(),
            "catalog company name is empty"
        );
        ensure!(
            !company.exchange.trim().is_empty(),
            "catalog exchange is empty"
        );
        ensure!(
            company.sic > 0,
            "catalog SIC is invalid for {}",
            company.symbol
        );
        ensure!(
            company.public_float.is_finite() && company.public_float > 0.0,
            "catalog public float is invalid for {}",
            company.symbol
        );
        ensure!(
            company
                .shares_outstanding
                .is_none_or(|shares| shares.is_finite() && shares > 0.0),
            "catalog shares outstanding is invalid for {}",
            company.symbol
        );
        ensure!(!company.as_of.is_empty(), "catalog as-of date is empty");
        ensure!(!company.quality.is_empty(), "catalog quality is empty");
        ensure!(symbols.insert(&company.symbol), "duplicate catalog symbol");
        ensure!(ciks.insert(&company.cik), "duplicate catalog CIK");
        ensure!(
            ranks
                .entry(company.sector)
                .or_default()
                .insert(company.rank),
            "duplicate rank in {}",
            company.sector
        );
    }
    for sector in Sector::ALL {
        let sector_ranks = ranks.get(&sector).context("catalog sector is missing")?;
        ensure!(
            (MIN_COMPANIES_PER_SECTOR..=MAX_COMPANIES_PER_SECTOR).contains(&sector_ranks.len())
                && (1..=u16::try_from(sector_ranks.len()).unwrap_or(u16::MAX))
                    .all(|rank| sector_ranks.contains(&rank)),
            "catalog sector {sector} must contain 100 to 250 consecutive ranks"
        );
    }

    Ok(catalog
        .companies
        .into_iter()
        .map(|entry| Company {
            symbol: entry.symbol,
            name: entry.name,
            sector: Some(entry.sector),
            raw_sector: Some(format!("SEC SIC {}", entry.sic)),
            exchange: entry.exchange,
            industry: format!("SEC SIC {}", entry.sic),
            market_cap: None,
            shares_outstanding: entry.shares_outstanding,
            rank: Some(entry.rank),
            description: format!(
                "SEC issuer CIK {}; catalog public float is a ranking proxy",
                entry.cik
            ),
            in_universe: false,
            retained: true,
            updated_at: now,
        })
        .collect())
}

/// Converts SEC share-class and preferred-share suffixes to Alpaca notation.
fn normalize_sec_symbol(symbol: &str) -> String {
    let Some((base, suffix)) = symbol.rsplit_once('-') else {
        return symbol.to_owned();
    };
    if let Some(series) = suffix.strip_prefix('P')
        && !series.is_empty()
    {
        format!("{base}.PR{series}")
    } else {
        format!("{base}.{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn embedded_catalog_has_one_hundred_unique_issuers_per_sector() {
        let now = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
        let companies = embedded_companies(now).expect("embedded catalog");
        assert!(companies.len() >= 900);
        assert!(companies.iter().all(|company| company.market_cap.is_none()));
        assert!(companies.iter().all(|company| !company.in_universe));
        assert!(companies.iter().all(|company| company.retained));
        assert!(companies.iter().all(|company| company.updated_at == now));
        assert!(
            companies
                .iter()
                .all(|company| !company.symbol.contains('-'))
        );
        assert!(companies.iter().any(|company| company.symbol == "BRK.B"));
        for sector in Sector::ALL {
            let count = companies
                .iter()
                .filter(|company| company.sector == Some(sector))
                .count();
            assert!((MIN_COMPANIES_PER_SECTOR..=MAX_COMPANIES_PER_SECTOR).contains(&count));
        }
    }

    #[test]
    fn sec_share_class_symbols_use_alpaca_notation() {
        assert_eq!(normalize_sec_symbol("BRK-B"), "BRK.B");
        assert_eq!(normalize_sec_symbol("TRTN-PA"), "TRTN.PRA");
        assert_eq!(normalize_sec_symbol("AAPL"), "AAPL");
    }
}
