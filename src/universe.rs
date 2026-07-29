//! Embedded and remotely refreshed SEC issuer universe.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, NaiveDate, Utc};
use futures_util::StreamExt;
use reqwest::{Client, Url};
use serde::Deserialize;

use crate::domain::{Company, Sector};

const CATALOG_JSON: &str = include_str!("../data/sec_universe.json");
const CATALOG_CACHE_FILE: &str = "catalog/sec_universe.json";
const MAX_CATALOG_BYTES: usize = 16 * 1024 * 1024;
const MIN_CATALOG_SCHEMA_VERSION: u32 = 1;
const MAX_CATALOG_SCHEMA_VERSION: u32 = 2;
const MIN_COMPANIES_PER_SECTOR: usize = 100;
const MAX_COMPANIES_PER_SECTOR: usize = 250;
const MAX_SYMBOL_LEN: usize = 16;
const MAX_NAME_LEN: usize = 160;
const MAX_METADATA_LEN: usize = 128;

#[derive(Debug, Deserialize)]
struct Catalog {
    schema_version: u32,
    #[serde(default)]
    catalog_version: Option<String>,
    #[serde(default)]
    generated_at: Option<String>,
    #[serde(default)]
    as_of: Option<String>,
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
    #[serde(default)]
    proxy_source: Option<String>,
    #[serde(default)]
    proxy_as_of: Option<String>,
    #[serde(default)]
    proxy_confidence: Option<String>,
    #[serde(default)]
    shares_source: Option<String>,
    #[serde(default)]
    shares_as_of: Option<String>,
    #[serde(default)]
    shares_method: Option<String>,
    #[serde(default)]
    shares_confidence: Option<String>,
    as_of: String,
    quality: String,
    provenance: CatalogProvenance,
}

#[derive(Debug, Deserialize)]
struct CatalogProvenance {
    public_float: CatalogFactProvenance,
    shares_outstanding: Option<CatalogSharesProvenance>,
}

#[derive(Debug, Deserialize)]
struct CatalogFactProvenance {
    source: String,
    end: String,
    #[serde(default)]
    confidence: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CatalogSharesProvenance {
    source: String,
    end: String,
    method: Option<String>,
    confidence: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogSource {
    Remote,
    Cache,
    Embedded,
}

impl CatalogSource {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Remote => "remote",
            Self::Cache => "cached",
            Self::Embedded => "embedded",
        }
    }
}

#[derive(Debug)]
pub struct LoadedCatalog {
    pub companies: Vec<Company>,
    pub source: CatalogSource,
    pub version: Option<String>,
}

#[derive(Debug)]
struct ParsedCatalog {
    companies: Vec<Company>,
    version: Option<String>,
    generated_at: Option<DateTime<Utc>>,
    as_of: Option<NaiveDate>,
}

impl ParsedCatalog {
    fn freshness(&self) -> (Option<NaiveDate>, Option<DateTime<Utc>>) {
        (self.as_of, self.generated_at)
    }

    fn has_plausible_remote_time(&self, now: DateTime<Utc>) -> bool {
        let latest = now + chrono::Duration::hours(24);
        self.generated_at.is_none_or(|value| value <= latest)
            && self.as_of.is_none_or(|value| value <= latest.date_naive())
    }
}

/// Loads and validates the build-time SEC catalog as runtime domain companies.
///
/// `EntityPublicFloat` is used only to establish the embedded ranks. It is not
/// copied into `Company::market_cap`; a market cap needs current price data.
pub fn embedded_companies(now: DateTime<Utc>) -> Result<Vec<Company>> {
    Ok(parse_catalog_json(CATALOG_JSON, now)?.companies)
}

/// Resolves the newest valid catalog without making offline startup depend on
/// the network. A fresh local copy avoids a request; all remote failures fall
/// back to the newest valid local or embedded catalog.
pub async fn load_companies(
    now: DateTime<Utc>,
    cache_dir: &Path,
    remote_url: Option<&str>,
    refresh_after: Duration,
) -> Result<LoadedCatalog> {
    let embedded = parse_catalog_json(CATALOG_JSON, now)
        .context("embedded SEC catalog could not be loaded")?;
    let cache_path = cache_dir.join(CATALOG_CACHE_FILE);
    let cached = load_cached_catalog(&cache_path, now);
    let cache_is_fresh = cached.is_some() && file_is_fresh(&cache_path, refresh_after);
    let cache_is_current = cached
        .as_ref()
        .is_some_and(|catalog| catalog.freshness() >= embedded.freshness());
    let should_fetch = remote_url.is_some() && (!cache_is_fresh || !cache_is_current);

    let remote = if should_fetch {
        let url = remote_url.expect("checked above");
        match fetch_catalog(url).await {
            Ok(contents) => match parse_catalog_json(&contents, now) {
                Ok(catalog) if !catalog.has_plausible_remote_time(Utc::now()) => {
                    tracing::warn!(url, "ignored SEC catalog with future metadata");
                    None
                }
                Ok(catalog)
                    if catalog.freshness()
                        >= cached
                            .as_ref()
                            .map_or(embedded.freshness(), ParsedCatalog::freshness)
                            .max(embedded.freshness()) =>
                {
                    if let Err(error) = store_cached_catalog(&cache_path, &contents) {
                        tracing::warn!(
                            path = %cache_path.display(),
                            error = %error,
                            "could not cache refreshed SEC catalog"
                        );
                    }
                    Some(catalog)
                }
                Ok(_) => {
                    tracing::warn!(url, "ignored an older remote SEC catalog");
                    None
                }
                Err(error) => {
                    tracing::warn!(
                        url,
                        error = %error,
                        "ignored an invalid remote SEC catalog"
                    );
                    None
                }
            },
            Err(error) => {
                tracing::warn!(
                    url,
                    error = %error,
                    "remote SEC catalog refresh failed"
                );
                None
            }
        }
    } else {
        None
    };

    let (catalog, source) = if let Some(remote) = remote {
        (remote, CatalogSource::Remote)
    } else if let Some(cached) = cached
        && cached.freshness() >= embedded.freshness()
    {
        (cached, CatalogSource::Cache)
    } else {
        (embedded, CatalogSource::Embedded)
    };
    Ok(LoadedCatalog {
        companies: catalog.companies,
        source,
        version: catalog.version,
    })
}

fn parse_catalog_json(catalog_json: &str, now: DateTime<Utc>) -> Result<ParsedCatalog> {
    let mut catalog: Catalog =
        serde_json::from_str(catalog_json).context("SEC catalog is invalid JSON")?;
    ensure!(
        catalog_schema_supported(catalog.schema_version),
        "unsupported SEC catalog schema {}",
        catalog.schema_version
    );
    let generated_at = catalog
        .generated_at
        .as_deref()
        .map(DateTime::parse_from_rfc3339)
        .transpose()
        .context("catalog generated_at is invalid")?
        .map(|value| value.with_timezone(&Utc));
    let as_of = catalog
        .as_of
        .as_deref()
        .map(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d"))
        .transpose()
        .context("catalog as_of is invalid")?;
    ensure!(
        catalog
            .catalog_version
            .as_deref()
            .is_none_or(|value| safe_text(value, MAX_METADATA_LEN)),
        "catalog version is invalid"
    );
    for company in &mut catalog.companies {
        company.symbol = normalize_sec_symbol(&company.symbol);
    }
    let mut symbols = HashSet::with_capacity(catalog.companies.len());
    let mut ciks = HashSet::with_capacity(catalog.companies.len());
    let mut ranks: HashMap<Sector, HashSet<u16>> = HashMap::new();
    for company in &catalog.companies {
        ensure!(valid_symbol(&company.symbol), "catalog symbol is invalid");
        ensure!(
            safe_text(&company.name, MAX_NAME_LEN),
            "catalog company name is invalid"
        );
        ensure!(
            safe_text(&company.exchange, MAX_METADATA_LEN),
            "catalog exchange is invalid"
        );
        ensure!(
            company.cik.len() == 10 && company.cik.bytes().all(|byte| byte.is_ascii_digit()),
            "catalog CIK is invalid for {}",
            company.symbol
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
        ensure!(
            NaiveDate::parse_from_str(&company.as_of, "%Y-%m-%d").is_ok(),
            "catalog as-of date is invalid for {}",
            company.symbol
        );
        ensure!(
            safe_text(&company.quality, MAX_METADATA_LEN),
            "catalog quality is invalid for {}",
            company.symbol
        );
        let proxy_source = company
            .proxy_source
            .as_deref()
            .unwrap_or(&company.provenance.public_float.source);
        ensure!(
            safe_text(proxy_source, MAX_METADATA_LEN),
            "catalog public-float source is empty for {}",
            company.symbol
        );
        let proxy_as_of = company
            .proxy_as_of
            .as_deref()
            .unwrap_or(&company.provenance.public_float.end);
        ensure!(
            NaiveDate::parse_from_str(proxy_as_of, "%Y-%m-%d").is_ok(),
            "catalog public-float date is invalid for {}",
            company.symbol
        );
        ensure!(
            company.shares_outstanding.is_some() == company.provenance.shares_outstanding.is_some(),
            "catalog shares provenance does not match shares for {}",
            company.symbol
        );
        if let Some(provenance) = &company.provenance.shares_outstanding {
            let shares_source = company
                .shares_source
                .as_deref()
                .unwrap_or(&provenance.source);
            ensure!(
                safe_text(shares_source, MAX_METADATA_LEN),
                "catalog shares source is empty for {}",
                company.symbol
            );
            let shares_as_of = company.shares_as_of.as_deref().unwrap_or(&provenance.end);
            ensure!(
                NaiveDate::parse_from_str(shares_as_of, "%Y-%m-%d").is_ok(),
                "catalog shares date is invalid for {}",
                company.symbol
            );
            ensure!(
                company
                    .shares_method
                    .as_deref()
                    .or(provenance.method.as_deref())
                    .is_none_or(|value| safe_text(value, MAX_METADATA_LEN)),
                "catalog shares method is invalid for {}",
                company.symbol
            );
            ensure!(
                company
                    .shares_confidence
                    .as_deref()
                    .or(provenance.confidence.as_deref())
                    .is_none_or(|value| safe_text(value, MAX_METADATA_LEN)),
                "catalog shares confidence is invalid for {}",
                company.symbol
            );
        }
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

    let companies = catalog
        .companies
        .into_iter()
        .map(|entry| {
            let proxy_as_of = entry
                .proxy_as_of
                .as_deref()
                .unwrap_or(&entry.provenance.public_float.end);
            let size_proxy_as_of = NaiveDate::parse_from_str(proxy_as_of, "%Y-%m-%d")
                .context("validated catalog public-float date became invalid")?;
            let shares_as_of_text = entry.shares_as_of.as_deref().or_else(|| {
                entry
                    .provenance
                    .shares_outstanding
                    .as_ref()
                    .map(|provenance| provenance.end.as_str())
            });
            let shares_as_of = entry
                .shares_outstanding
                .zip(shares_as_of_text)
                .map(|(_, as_of)| {
                    NaiveDate::parse_from_str(as_of, "%Y-%m-%d")
                        .context("validated catalog shares date became invalid")
                })
                .transpose()?;
            Ok(Company {
                symbol: entry.symbol,
                name: entry.name,
                sector: Some(entry.sector),
                raw_sector: Some(format!("SEC SIC {}", entry.sic)),
                exchange: entry.exchange,
                industry: format!("SEC SIC {}", entry.sic),
                market_cap: None,
                size_proxy: Some(entry.public_float),
                size_proxy_source: entry
                    .proxy_source
                    .or(Some(entry.provenance.public_float.source)),
                size_proxy_as_of: Some(size_proxy_as_of),
                size_proxy_confidence: entry
                    .proxy_confidence
                    .or(entry.provenance.public_float.confidence)
                    .or_else(|| Some("low".to_owned())),
                shares_outstanding: entry.shares_outstanding,
                shares_source: entry.shares_source.or_else(|| {
                    entry
                        .provenance
                        .shares_outstanding
                        .as_ref()
                        .map(|provenance| provenance.source.clone())
                }),
                shares_as_of,
                shares_method: entry
                    .shares_method
                    .or_else(|| {
                        entry
                            .provenance
                            .shares_outstanding
                            .as_ref()
                            .and_then(|provenance| provenance.method.clone())
                    })
                    .or_else(|| entry.shares_outstanding.map(|_| "sec_frame".to_owned())),
                shares_confidence: entry
                    .shares_confidence
                    .or_else(|| {
                        entry
                            .provenance
                            .shares_outstanding
                            .as_ref()
                            .and_then(|provenance| provenance.confidence.clone())
                    })
                    .or_else(|| entry.shares_outstanding.map(|_| "high".to_owned())),
                rank: Some(entry.rank),
                description: format!(
                    "SEC issuer CIK {}; catalog public float is a ranking proxy",
                    entry.cik
                ),
                in_universe: false,
                retained: true,
                updated_at: now,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ParsedCatalog {
        companies,
        version: catalog.catalog_version,
        generated_at,
        as_of,
    })
}

#[cfg(test)]
fn companies_from_catalog_json(catalog_json: &str, now: DateTime<Utc>) -> Result<Vec<Company>> {
    Ok(parse_catalog_json(catalog_json, now)?.companies)
}

fn load_cached_catalog(path: &Path, now: DateTime<Utc>) -> Option<ParsedCatalog> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "could not inspect cached SEC catalog"
            );
            return None;
        }
    };
    if metadata.len() > u64::try_from(MAX_CATALOG_BYTES).unwrap_or(u64::MAX) {
        tracing::warn!(
            path = %path.display(),
            bytes = metadata.len(),
            "ignored oversized cached SEC catalog"
        );
        return None;
    }
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "could not read cached SEC catalog"
            );
            return None;
        }
    };
    match parse_catalog_json(&contents, now) {
        Ok(catalog) if catalog.has_plausible_remote_time(Utc::now()) => Some(catalog),
        Ok(_) => {
            tracing::warn!(
                path = %path.display(),
                "ignored cached SEC catalog with future metadata"
            );
            None
        }
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "ignored invalid cached SEC catalog"
            );
            None
        }
    }
}

fn file_is_fresh(path: &Path, max_age: Duration) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
        .is_ok_and(|age| age <= max_age)
}

async fn fetch_catalog(url: &str) -> Result<String> {
    let parsed = Url::parse(url).context("catalog URL is invalid")?;
    let is_loopback_http = parsed.scheme() == "http"
        && parsed.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
    ensure!(
        parsed.scheme() == "https" || is_loopback_http,
        "catalog URL must use HTTPS unless it targets loopback"
    );
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = Client::builder()
        .user_agent(concat!("stock-tui/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(10))
        .build()
        .context("could not initialize catalog HTTP client")?;
    let response = client
        .get(parsed)
        .send()
        .await
        .context("could not reach catalog service")?
        .error_for_status()
        .context("catalog service returned an error")?;
    if response
        .content_length()
        .is_some_and(|length| length > u64::try_from(MAX_CATALOG_BYTES).unwrap_or(u64::MAX))
    {
        anyhow::bail!("catalog response exceeds the size limit");
    }
    let mut contents = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("could not read catalog response")?;
        ensure!(
            contents.len().saturating_add(chunk.len()) <= MAX_CATALOG_BYTES,
            "catalog response exceeds the size limit"
        );
        contents.extend_from_slice(&chunk);
    }
    String::from_utf8(contents).context("catalog response is not UTF-8")
}

fn store_cached_catalog(path: &Path, contents: &str) -> Result<()> {
    ensure!(
        contents.len() <= MAX_CATALOG_BYTES,
        "catalog exceeds the cache size limit"
    );
    let parent = path.parent().context("catalog cache path has no parent")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("could not create catalog cache at {}", parent.display()))?;
    let temporary = temporary_catalog_path(path);
    fs::write(&temporary, contents)
        .with_context(|| format!("could not write catalog cache at {}", temporary.display()))?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)
            .with_context(|| format!("could not replace catalog cache at {}", path.display()))?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error)
            .with_context(|| format!("could not install catalog cache at {}", path.display()));
    }
    Ok(())
}

fn temporary_catalog_path(path: &Path) -> PathBuf {
    path.with_extension(format!("json.{}.tmp", std::process::id()))
}

fn safe_text(value: &str, max_chars: usize) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= max_chars
        && !value.chars().any(char::is_control)
}

fn valid_symbol(symbol: &str) -> bool {
    !symbol.is_empty()
        && symbol.len() <= MAX_SYMBOL_LEN
        && symbol.bytes().all(|byte| {
            byte.is_ascii_uppercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'$')
        })
}

const fn catalog_schema_supported(version: u32) -> bool {
    version >= MIN_CATALOG_SCHEMA_VERSION && version <= MAX_CATALOG_SCHEMA_VERSION
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

/// Return a newer catalog canonical for a retired display symbol.
///
/// These are catalog membership replacements, not interchangeable market-data
/// aliases: the retired security remains independently searchable and usable.
pub(crate) fn catalog_symbol_replacement(symbol: &str) -> Option<&'static str> {
    match symbol {
        "GOOGL" => Some("GOOG"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use chrono::TimeZone;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn embedded_catalog_has_one_hundred_unique_issuers_per_sector() {
        let now = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
        let companies = embedded_companies(now).expect("embedded catalog");
        assert!(companies.len() >= 900);
        assert!(companies.iter().all(|company| company.market_cap.is_none()));
        assert!(
            companies
                .iter()
                .all(|company| company.size_proxy.is_some_and(|value| value > 0.0))
        );
        assert!(companies.iter().all(|company| {
            company.size_proxy_source.is_some()
                && company.size_proxy_as_of.is_some()
                && company.size_proxy_confidence.as_deref() == Some("low")
        }));
        assert!(companies.iter().all(|company| {
            company.shares_outstanding.is_some()
                == (company.shares_source.is_some()
                    && company.shares_as_of.is_some()
                    && company.shares_method.is_some()
                    && company.shares_confidence.is_some())
        }));
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
    fn embedded_catalog_includes_reviewed_dell_common_shares() {
        let companies = embedded_companies(Utc::now()).expect("embedded catalog");
        let dell = companies
            .iter()
            .find(|company| company.symbol == "DELL")
            .expect("Dell catalog entry");

        assert_eq!(dell.shares_outstanding, Some(648_107_991.0));
        assert_eq!(dell.shares_as_of, NaiveDate::from_ymd_opt(2026, 6, 2));
        assert_eq!(
            dell.shares_method.as_deref(),
            Some("filing_cover_reviewed_policy")
        );
        assert_eq!(dell.shares_confidence.as_deref(), Some("medium"));
    }

    #[test]
    fn sec_share_class_symbols_use_alpaca_notation() {
        assert_eq!(normalize_sec_symbol("BRK-B"), "BRK.B");
        assert_eq!(normalize_sec_symbol("TRTN-PA"), "TRTN.PRA");
        assert_eq!(normalize_sec_symbol("AAPL"), "AAPL");
    }

    #[test]
    fn retired_alphabet_catalog_symbol_points_to_the_concise_class() {
        assert_eq!(catalog_symbol_replacement("GOOGL"), Some("GOOG"));
        assert_eq!(catalog_symbol_replacement("GOOG"), None);
    }

    #[test]
    fn catalog_versions_are_supported_only_through_v2() {
        assert!(catalog_schema_supported(1));
        assert!(catalog_schema_supported(2));
        assert!(!catalog_schema_supported(0));
        assert!(!catalog_schema_supported(3));
    }

    #[test]
    fn schema_v1_catalog_shape_loads_through_legacy_provenance_fallbacks() {
        let mut value: serde_json::Value =
            serde_json::from_str(CATALOG_JSON).expect("committed catalog");
        value["schema_version"] = serde_json::json!(1);
        let companies = value["companies"]
            .as_array_mut()
            .expect("catalog companies");
        let shares_symbol = companies
            .iter()
            .find(|company| !company["shares_outstanding"].is_null())
            .and_then(|company| company["symbol"].as_str())
            .expect("catalog company with shares")
            .to_owned();
        for company in companies {
            let object = company.as_object_mut().expect("catalog company object");
            for field in [
                "proxy_source",
                "proxy_as_of",
                "proxy_confidence",
                "shares_source",
                "shares_as_of",
                "shares_method",
                "shares_confidence",
            ] {
                object.remove(field);
            }
            if let Some(public_float) = object
                .get_mut("provenance")
                .and_then(serde_json::Value::as_object_mut)
                .and_then(|provenance| provenance.get_mut("public_float"))
                .and_then(serde_json::Value::as_object_mut)
            {
                public_float.remove("confidence");
            }
            if let Some(shares) = object
                .get_mut("provenance")
                .and_then(serde_json::Value::as_object_mut)
                .and_then(|provenance| provenance.get_mut("shares_outstanding"))
                .and_then(serde_json::Value::as_object_mut)
            {
                shares.remove("method");
                shares.remove("confidence");
            }
        }

        let now = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
        let companies = companies_from_catalog_json(&value.to_string(), now)
            .expect("schema-v1 catalog remains loadable");
        let company = companies
            .iter()
            .find(|company| company.symbol == shares_symbol)
            .expect("company with legacy shares remains present");
        assert_eq!(company.shares_method.as_deref(), Some("sec_frame"));
        assert_eq!(company.shares_confidence.as_deref(), Some("high"));
        assert_eq!(company.size_proxy_confidence.as_deref(), Some("low"));
    }

    #[test]
    fn schema_v2_top_level_share_method_overrides_nested_provenance() {
        let mut value: serde_json::Value =
            serde_json::from_str(CATALOG_JSON).expect("committed catalog");
        value["schema_version"] = serde_json::json!(2);
        let company = value["companies"]
            .as_array_mut()
            .and_then(|companies| {
                companies
                    .iter_mut()
                    .find(|company| !company["shares_outstanding"].is_null())
            })
            .expect("catalog company with shares");
        let symbol = company["symbol"]
            .as_str()
            .expect("catalog symbol")
            .to_owned();
        company["shares_method"] = serde_json::json!("top_level_method");

        let now = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
        let companies =
            companies_from_catalog_json(&value.to_string(), now).expect("schema-v2 catalog");
        let company = companies
            .iter()
            .find(|company| company.symbol == symbol)
            .expect("modified company remains present");
        assert_eq!(company.shares_method.as_deref(), Some("top_level_method"));
    }

    #[test]
    fn compact_runtime_catalog_uses_nested_provenance_fallbacks() {
        let mut value: serde_json::Value =
            serde_json::from_str(CATALOG_JSON).expect("committed catalog");
        let object = value.as_object_mut().expect("catalog object");
        object.remove("selection");
        object.remove("sources");
        for company in object["companies"]
            .as_array_mut()
            .expect("catalog companies")
        {
            let company = company.as_object_mut().expect("catalog company object");
            for field in [
                "proxy_source",
                "proxy_as_of",
                "proxy_confidence",
                "proxy_sanity_screen",
                "shares_source",
                "shares_as_of",
                "shares_method",
                "shares_confidence",
            ] {
                company.remove(field);
            }
        }

        let now = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
        let companies = companies_from_catalog_json(&value.to_string(), now)
            .expect("compact catalog remains loadable");
        assert!(companies.len() >= 900);
        assert!(
            companies
                .iter()
                .any(|company| company.shares_outstanding.is_some())
        );
    }

    #[tokio::test]
    async fn offline_resolution_prefers_a_newer_valid_cache() {
        let directory = tempdir().expect("temporary directory");
        let cache_path = directory.path().join(CATALOG_CACHE_FILE);
        fs::create_dir_all(cache_path.parent().expect("cache parent")).expect("cache directory");
        let mut value: serde_json::Value =
            serde_json::from_str(CATALOG_JSON).expect("committed catalog");
        let future = Utc::now() + chrono::Duration::minutes(1);
        value["generated_at"] = serde_json::json!(future.to_rfc3339());
        value["as_of"] = serde_json::json!(future.date_naive().to_string());
        fs::write(&cache_path, value.to_string()).expect("cached catalog");

        let loaded = load_companies(
            Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap(),
            directory.path(),
            None,
            Duration::from_secs(1),
        )
        .await
        .expect("catalog resolution");

        assert_eq!(loaded.source, CatalogSource::Cache);
        assert!(loaded.companies.len() >= 900);
    }

    #[tokio::test]
    async fn remote_resolution_installs_a_newer_valid_catalog() {
        let directory = tempdir().expect("temporary directory");
        let now = Utc::now();
        let mut value: serde_json::Value =
            serde_json::from_str(CATALOG_JSON).expect("committed catalog");
        value["catalog_version"] = serde_json::json!("remote-test");
        value["generated_at"] =
            serde_json::json!((now + chrono::Duration::minutes(1)).to_rfc3339());
        value["as_of"] = serde_json::json!(now.date_naive().to_string());
        let body = value.to_string();

        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
        let address = listener.local_addr().expect("listener address");
        let server = thread::spawn(move || {
            let (mut connection, _) = listener.accept().expect("catalog request");
            let mut request = [0_u8; 4096];
            let _ = connection.read(&mut request).expect("request bytes");
            write!(
                connection,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("catalog response");
        });

        let loaded = load_companies(
            now,
            directory.path(),
            Some(&format!("http://{address}/catalog.json")),
            Duration::from_secs(1),
        )
        .await
        .expect("remote catalog resolution");
        server.join().expect("catalog server");

        assert_eq!(loaded.source, CatalogSource::Remote);
        assert_eq!(loaded.version.as_deref(), Some("remote-test"));
        assert!(directory.path().join(CATALOG_CACHE_FILE).is_file());
    }

    #[tokio::test]
    async fn invalid_cache_falls_back_to_the_embedded_catalog() {
        let directory = tempdir().expect("temporary directory");
        let cache_path = directory.path().join(CATALOG_CACHE_FILE);
        fs::create_dir_all(cache_path.parent().expect("cache parent")).expect("cache directory");
        fs::write(&cache_path, b"{not-json").expect("invalid cached catalog");

        let loaded = load_companies(
            Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap(),
            directory.path(),
            None,
            Duration::from_secs(1),
        )
        .await
        .expect("catalog resolution");

        assert_eq!(loaded.source, CatalogSource::Embedded);
        assert!(loaded.companies.len() >= 900);
    }

    #[tokio::test]
    async fn terminal_control_characters_make_a_cached_catalog_invalid() {
        let directory = tempdir().expect("temporary directory");
        let cache_path = directory.path().join(CATALOG_CACHE_FILE);
        fs::create_dir_all(cache_path.parent().expect("cache parent")).expect("cache directory");
        let mut value: serde_json::Value =
            serde_json::from_str(CATALOG_JSON).expect("committed catalog");
        value["companies"][0]["name"] = serde_json::json!("unsafe\u{1b}[2Jname");
        fs::write(&cache_path, value.to_string()).expect("unsafe cached catalog");

        let loaded = load_companies(Utc::now(), directory.path(), None, Duration::from_secs(1))
            .await
            .expect("catalog resolution");

        assert_eq!(loaded.source, CatalogSource::Embedded);
    }

    #[tokio::test]
    async fn far_future_cache_cannot_pin_catalog_freshness() {
        let directory = tempdir().expect("temporary directory");
        let cache_path = directory.path().join(CATALOG_CACHE_FILE);
        fs::create_dir_all(cache_path.parent().expect("cache parent")).expect("cache directory");
        let mut value: serde_json::Value =
            serde_json::from_str(CATALOG_JSON).expect("committed catalog");
        value["generated_at"] = serde_json::json!("2099-01-02T00:00:00Z");
        value["as_of"] = serde_json::json!("2099-01-01");
        fs::write(&cache_path, value.to_string()).expect("future cached catalog");

        let loaded = load_companies(Utc::now(), directory.path(), None, Duration::from_secs(1))
            .await
            .expect("catalog resolution");

        assert_eq!(loaded.source, CatalogSource::Embedded);
    }
}
