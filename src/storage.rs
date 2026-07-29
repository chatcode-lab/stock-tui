use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use rusqlite::{
    Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params, types::Type,
};

use crate::{
    benchmarks::MarketBenchmark,
    domain::{
        Bar, Company, DateRange, MarketTile, NewsItem, Sector, Snapshot, SortMode, TickerDetail,
    },
    market::{CacheIdentity, MarketContext},
};

const SCHEMA_VERSION: i64 = 3;
const STALE_AFTER_HOURS: i64 = 72;
const MAX_MEMBERS_PER_SECTOR: usize = 100;
const TIMEFRAME_EXISTS_SQL: &str = "
    SELECT EXISTS(
        SELECT 1 FROM bars
        WHERE symbol = ?1 AND timeframe = ?2
          AND NOT (
              volume = 0 AND COALESCE(trade_count, 0) = 0
              AND open = high AND high = low AND low = close
          )
        LIMIT 1
    )";
const PERIOD_METRIC_SQL: &str = "WITH
    before_cutoff AS (
        SELECT close, timestamp FROM bars
        WHERE symbol = ?1 AND timeframe = ?2 AND timestamp <= ?3
          AND NOT (
              volume = 0 AND COALESCE(trade_count, 0) = 0
              AND open = high AND high = low AND low = close
          )
        ORDER BY timestamp DESC LIMIT 1
    ),
    after_cutoff AS (
        SELECT close, timestamp FROM bars
        WHERE symbol = ?1 AND timeframe = ?2
          AND timestamp >= ?3 AND timestamp <= ?4
          AND NOT (
              volume = 0 AND COALESCE(trade_count, 0) = 0
              AND open = high AND high = low AND low = close
          )
        ORDER BY timestamp ASC LIMIT 1
    ),
    latest AS (
        SELECT close, timestamp FROM bars
        WHERE symbol = ?1 AND timeframe = ?2 AND timestamp <= ?4
          AND NOT (
              volume = 0 AND COALESCE(trade_count, 0) = 0
              AND open = high AND high = low AND low = close
          )
        ORDER BY timestamp DESC LIMIT 1
    )
SELECT
    COALESCE(
        (SELECT close FROM before_cutoff),
        (SELECT close FROM after_cutoff)
    ),
    COALESCE(
        (SELECT timestamp FROM before_cutoff),
        (SELECT timestamp FROM after_cutoff)
    ),
    (SELECT close FROM latest),
    (SELECT timestamp FROM latest)";
const PERIOD_VOLUME_SQL: &str = "
    SELECT SUM(volume) FROM bars
    WHERE symbol = ?1 AND timeframe = ?2
      AND timestamp >= ?3 AND timestamp <= ?4
      AND volume >= 0";

const COMPANY_COLUMNS: &str = "
    symbol, name, sector, raw_sector, exchange, industry, market_cap,
    size_proxy, size_proxy_source, size_proxy_as_of, size_proxy_confidence,
    shares_outstanding, shares_source, shares_as_of, shares_method, shares_confidence,
    rank, description, in_universe, retained, updated_at
";

const MIGRATION_1: &str = r#"
CREATE TABLE IF NOT EXISTS companies (
    symbol TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    sector TEXT,
    raw_sector TEXT,
    exchange TEXT NOT NULL DEFAULT '',
    industry TEXT NOT NULL DEFAULT '',
    market_cap REAL,
    shares_outstanding REAL,
    rank INTEGER,
    description TEXT NOT NULL DEFAULT '',
    in_universe INTEGER NOT NULL DEFAULT 1 CHECK (in_universe IN (0, 1)),
    retained INTEGER NOT NULL DEFAULT 0 CHECK (retained IN (0, 1)),
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS companies_by_sector
    ON companies (sector, in_universe, rank, market_cap DESC);
CREATE INDEX IF NOT EXISTS companies_by_name
    ON companies (name COLLATE NOCASE);

CREATE TABLE IF NOT EXISTS sector_memberships (
    as_of_date TEXT NOT NULL,
    sector TEXT NOT NULL,
    symbol TEXT NOT NULL REFERENCES companies(symbol) ON DELETE CASCADE,
    rank INTEGER NOT NULL,
    market_cap REAL,
    PRIMARY KEY (as_of_date, sector, symbol)
);

CREATE INDEX IF NOT EXISTS memberships_by_sector_date
    ON sector_memberships (sector, as_of_date DESC, rank);
CREATE INDEX IF NOT EXISTS memberships_by_symbol
    ON sector_memberships (symbol, as_of_date DESC);

CREATE TABLE IF NOT EXISTS bars (
    symbol TEXT NOT NULL REFERENCES companies(symbol) ON DELETE CASCADE,
    timeframe TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    open REAL NOT NULL,
    high REAL NOT NULL,
    low REAL NOT NULL,
    close REAL NOT NULL,
    volume REAL NOT NULL,
    trade_count INTEGER,
    vwap REAL,
    source TEXT NOT NULL DEFAULT 'alpaca',
    PRIMARY KEY (symbol, timeframe, timestamp)
);

CREATE INDEX IF NOT EXISTS bars_by_symbol_time
    ON bars (symbol, timeframe, timestamp DESC);

CREATE TABLE IF NOT EXISTS snapshots (
    symbol TEXT PRIMARY KEY REFERENCES companies(symbol) ON DELETE CASCADE,
    price REAL,
    previous_close REAL,
    open REAL,
    high REAL,
    low REAL,
    volume REAL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS snapshots_by_update
    ON snapshots (updated_at DESC);

CREATE TABLE IF NOT EXISTS news (
    id TEXT PRIMARY KEY,
    headline TEXT NOT NULL,
    source TEXT NOT NULL,
    published_at INTEGER NOT NULL,
    url TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS news_by_publication
    ON news (published_at DESC);

CREATE TABLE IF NOT EXISTS news_symbols (
    news_id TEXT NOT NULL REFERENCES news(id) ON DELETE CASCADE,
    symbol TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (news_id, symbol)
);

CREATE INDEX IF NOT EXISTS news_symbols_by_symbol
    ON news_symbols (symbol, news_id);

CREATE TABLE IF NOT EXISTS favorites (
    symbol TEXT PRIMARY KEY REFERENCES companies(symbol) ON DELETE CASCADE,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sync_checkpoints (
    scope TEXT PRIMARY KEY,
    completed_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
"#;

const MIGRATION_2: &str = r#"
ALTER TABLE companies ADD COLUMN size_proxy REAL;
ALTER TABLE companies ADD COLUMN size_proxy_source TEXT;
ALTER TABLE companies ADD COLUMN size_proxy_as_of TEXT;
ALTER TABLE companies ADD COLUMN size_proxy_confidence TEXT;
ALTER TABLE companies ADD COLUMN shares_source TEXT;
ALTER TABLE companies ADD COLUMN shares_as_of TEXT;
ALTER TABLE companies ADD COLUMN shares_method TEXT;
ALTER TABLE companies ADD COLUMN shares_confidence TEXT;
ALTER TABLE sector_memberships ADD COLUMN size_proxy REAL;
"#;

const MIGRATION_3: &str = r#"
CREATE TABLE IF NOT EXISTS cache_context (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    namespace TEXT NOT NULL,
    market_id TEXT NOT NULL,
    symbol_namespace TEXT NOT NULL,
    currency TEXT NOT NULL,
    timezone TEXT NOT NULL,
    regular_open TEXT NOT NULL,
    regular_close TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
"#;

#[derive(Debug, Clone)]
pub struct Storage {
    path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePreparation {
    Initialized,
    Reused,
    Reset,
}

impl CachePreparation {
    #[must_use]
    pub const fn was_reset(self) -> bool {
        matches!(self, Self::Reset)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StorageCounts {
    pub companies: usize,
    pub memberships: usize,
    pub bars: usize,
    pub snapshots: usize,
    pub news: usize,
    pub favorites: usize,
    pub checkpoints: usize,
}

#[derive(Debug, Clone, Copy)]
struct PeriodMetric {
    baseline: Option<f64>,
    baseline_at: Option<DateTime<Utc>>,
    close: Option<f64>,
    updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndpointSource {
    Snapshot,
    Bar,
}

#[derive(Debug, Clone, Copy)]
struct ResolvedPeriod {
    price: Option<f64>,
    baseline: Option<f64>,
    baseline_at: Option<DateTime<Utc>>,
    period_return: Option<f64>,
    updated_at: Option<DateTime<Utc>>,
    source: Option<EndpointSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredCacheIdentity {
    namespace: String,
    market_id: String,
    symbol_namespace: String,
    currency: String,
    timezone: String,
    regular_open: String,
    regular_close: String,
}

type PeriodMetricRow = (Option<f64>, Option<i64>, Option<f64>, Option<i64>);
type PriceHistoryCoverage = (Option<DateTime<Utc>>, Option<DateTime<Utc>>);

impl Storage {
    /// Opens a path-backed cache and applies all known schema migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if path == Path::new(":memory:") {
            bail!("Storage requires a file path because connections are short-lived");
        }
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("could not create database directory {}", parent.display())
            })?;
        }
        let storage = Self { path };
        storage.migrate()?;
        Ok(storage)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn connection(&self) -> Result<Connection> {
        let connection = Connection::open(&self.path)
            .with_context(|| format!("could not open SQLite cache at {}", self.path.display()))?;
        connection
            .busy_timeout(Duration::from_secs(30))
            .context("could not configure SQLite busy timeout")?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .context("could not enable SQLite foreign keys")?;
        Ok(connection)
    }

    fn migrate(&self) -> Result<()> {
        let connection = self.connection()?;
        connection
            .execute_batch("PRAGMA journal_mode = WAL;")
            .context("could not enable SQLite WAL mode")?;
        let current: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .context("could not read SQLite schema version")?;
        if current > SCHEMA_VERSION {
            bail!(
                "database schema version {current} is newer than supported version {SCHEMA_VERSION}"
            );
        }
        if current < 1 {
            connection
                .execute_batch(&format!(
                    "BEGIN IMMEDIATE;\n{MIGRATION_1}\nPRAGMA user_version = 1;\nCOMMIT;"
                ))
                .context("could not apply SQLite schema migration 1")?;
        }
        if current < 2 {
            connection
                .execute_batch(&format!(
                    "BEGIN IMMEDIATE;\n{MIGRATION_2}\nPRAGMA user_version = 2;\nCOMMIT;"
                ))
                .context("could not apply SQLite schema migration 2")?;
        }
        if current < 3 {
            connection
                .execute_batch(&format!(
                    "BEGIN IMMEDIATE;\n{MIGRATION_3}\nPRAGMA user_version = 3;\nCOMMIT;"
                ))
                .context("could not apply SQLite schema migration 3")?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<i64> {
        self.connection()?
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .context("could not read SQLite schema version")
    }

    pub fn journal_mode(&self) -> Result<String> {
        self.connection()?
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .context("could not read SQLite journal mode")
    }

    /// Stamps a live cache with the active provider dataset and market context.
    ///
    /// A different provider, endpoint, feed, symbol namespace, or session profile
    /// invalidates unscoped rows. Favorite symbols are retained as neutral company
    /// records so the next synchronization can hydrate them again.
    pub fn prepare_live_cache(&self, identity: &CacheIdentity) -> Result<CachePreparation> {
        validate_cache_identity(identity)?;
        let expected = StoredCacheIdentity::from(identity);
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("could not begin live cache preparation")?;
        let current = load_stored_cache_identity(&transaction)?;
        if current.as_ref() == Some(&expected) {
            transaction
                .commit()
                .context("could not finish live cache inspection")?;
            return Ok(CachePreparation::Reused);
        }

        let has_cached_rows: bool = transaction.query_row(
            "SELECT
                EXISTS(SELECT 1 FROM companies LIMIT 1)
                OR EXISTS(SELECT 1 FROM sector_memberships LIMIT 1)
                OR EXISTS(SELECT 1 FROM bars LIMIT 1)
                OR EXISTS(SELECT 1 FROM snapshots LIMIT 1)
                OR EXISTS(SELECT 1 FROM news LIMIT 1)
                OR EXISTS(SELECT 1 FROM sync_checkpoints LIMIT 1)",
            [],
            |row| row.get(0),
        )?;
        let preparation = if current.is_some() || has_cached_rows {
            clear_incompatible_live_rows(&transaction)?;
            CachePreparation::Reset
        } else {
            CachePreparation::Initialized
        };
        store_cache_identity(&transaction, &expected)?;
        transaction
            .commit()
            .context("could not commit live cache preparation")?;
        Ok(preparation)
    }

    pub fn cache_identity(&self) -> Result<Option<CacheIdentity>> {
        let connection = self.connection()?;
        load_stored_cache_identity(&connection)?
            .map(StoredCacheIdentity::into_domain)
            .transpose()
    }

    pub fn market_context(&self) -> Result<Option<MarketContext>> {
        Ok(self.cache_identity()?.map(|identity| identity.market))
    }

    pub fn upsert_companies(&self, companies: &[Company]) -> Result<usize> {
        if companies.is_empty() {
            return Ok(0);
        }
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("could not begin company update")?;
        for company in companies {
            upsert_company(&transaction, company, None)?;
        }
        transaction
            .commit()
            .context("could not commit company update")?;
        Ok(companies.len())
    }

    pub fn company(&self, symbol: &str) -> Result<Option<Company>> {
        let connection = self.connection()?;
        connection
            .query_row(
                &format!("SELECT {COMPANY_COLUMNS} FROM companies WHERE symbol = ?1"),
                [normalize_symbol(symbol)?],
                company_from_row,
            )
            .optional()
            .context("could not load company")
    }

    pub fn companies(&self, sector: Option<Sector>, universe_only: bool) -> Result<Vec<Company>> {
        let connection = self.connection()?;
        let sector = sector.map(sector_key);
        let mut statement = connection.prepare(&format!(
            "SELECT {COMPANY_COLUMNS} FROM companies
             WHERE (?1 IS NULL OR sector = ?1)
               AND (?2 = 0 OR in_universe = 1)
             ORDER BY rank IS NULL, rank, market_cap IS NULL, market_cap DESC, symbol"
        ))?;
        let rows = statement.query_map(params![sector, universe_only], company_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("could not load companies")
    }

    pub fn replace_memberships(
        &self,
        as_of: NaiveDate,
        sector: Sector,
        companies: &[Company],
    ) -> Result<usize> {
        let selected = selected_members(companies, MAX_MEMBERS_PER_SECTOR);
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("could not begin membership update")?;
        transaction.execute(
            "UPDATE companies SET in_universe = 0 WHERE sector = ?1",
            [sector_key(sector)],
        )?;
        for company in &selected {
            upsert_company(&transaction, company, Some(true))?;
        }
        replace_sector_memberships(&transaction, as_of, sector, &selected)?;
        transaction
            .commit()
            .context("could not commit membership update")?;
        Ok(selected.len())
    }

    pub fn replace_universe(&self, as_of: NaiveDate, companies: &[Company]) -> Result<usize> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("could not begin universe update")?;
        transaction.execute("UPDATE companies SET in_universe = 0", [])?;
        for company in companies {
            upsert_company(&transaction, company, Some(true))?;
        }
        transaction.execute(
            "DELETE FROM sector_memberships WHERE as_of_date = ?1",
            [as_of.to_string()],
        )?;
        for sector in Sector::ALL {
            let sector_companies = companies
                .iter()
                .filter(|company| company.sector == Some(sector))
                .collect::<Vec<_>>();
            let selected = selected_members_from_refs(&sector_companies, MAX_MEMBERS_PER_SECTOR);
            insert_sector_memberships(&transaction, as_of, sector, &selected)?;
        }
        transaction
            .commit()
            .context("could not commit universe update")?;
        Ok(companies.len())
    }

    pub fn latest_membership_date(&self, sector: Option<Sector>) -> Result<Option<NaiveDate>> {
        let connection = self.connection()?;
        let value: Option<String> = connection.query_row(
            "SELECT MAX(as_of_date) FROM sector_memberships
             WHERE (?1 IS NULL OR sector = ?1)",
            [sector.map(sector_key)],
            |row| row.get(0),
        )?;
        value
            .map(|value| {
                NaiveDate::parse_from_str(&value, "%Y-%m-%d")
                    .with_context(|| format!("invalid membership date {value:?}"))
            })
            .transpose()
    }

    pub fn memberships(&self, sector: Sector, as_of: Option<NaiveDate>) -> Result<Vec<Company>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT c.symbol, c.name, m.sector, c.raw_sector, c.exchange, c.industry,
                    m.market_cap, m.size_proxy, c.size_proxy_source,
                    c.size_proxy_as_of, c.size_proxy_confidence,
                    c.shares_outstanding, c.shares_source, c.shares_as_of,
                    c.shares_method, c.shares_confidence,
                    m.rank, c.description, c.in_universe, c.retained, c.updated_at
             FROM sector_memberships m
             JOIN companies c ON c.symbol = m.symbol
             WHERE m.sector = ?1
               AND m.as_of_date = (
                    SELECT MAX(as_of_date) FROM sector_memberships
                    WHERE sector = ?1 AND (?2 IS NULL OR as_of_date <= ?2)
               )
             ORDER BY m.rank, c.symbol",
        )?;
        let rows = statement.query_map(
            params![sector_key(sector), as_of.map(|date| date.to_string())],
            company_from_row,
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("could not load sector memberships")
    }

    pub fn upsert_bars(&self, bars: &[Bar]) -> Result<usize> {
        self.upsert_bars_until_cancelled(bars, || false)
            .map(|count| count.expect("unconditional bar updates cannot cancel"))
    }

    pub(crate) fn upsert_bars_until_cancelled(
        &self,
        bars: &[Bar],
        mut is_cancelled: impl FnMut() -> bool,
    ) -> Result<Option<usize>> {
        if bars.is_empty() {
            return Ok(Some(0));
        }
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("could not begin bar update")?;
        {
            let mut statement = transaction.prepare_cached(
                "INSERT INTO bars (
                    symbol, timeframe, timestamp, open, high, low, close,
                    volume, trade_count, vwap, source
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(symbol, timeframe, timestamp) DO UPDATE SET
                    open = excluded.open,
                    high = excluded.high,
                    low = excluded.low,
                    close = excluded.close,
                    volume = excluded.volume,
                    trade_count = excluded.trade_count,
                    vwap = excluded.vwap,
                    source = excluded.source",
            )?;
            for bar in bars {
                if is_cancelled() {
                    return Ok(None);
                }
                statement.execute(params![
                    normalize_symbol(&bar.symbol)?,
                    bar.timeframe,
                    timestamp_millis(bar.timestamp),
                    bar.open,
                    bar.high,
                    bar.low,
                    bar.close,
                    bar.volume,
                    optional_u64_to_i64(bar.trade_count)?,
                    bar.vwap,
                    bar.source,
                ])?;
            }
        }
        transaction
            .commit()
            .context("could not commit bar update")?;
        Ok(Some(bars.len()))
    }

    pub fn bars(
        &self,
        symbol: &str,
        timeframe: Option<&str>,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        limit: Option<usize>,
    ) -> Result<Vec<Bar>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT symbol, timeframe, timestamp, open, high, low, close,
                    volume, trade_count, vwap, source
             FROM bars
             WHERE symbol = ?1
               AND (?2 IS NULL OR timeframe = ?2)
               AND (?3 IS NULL OR timestamp >= ?3)
               AND (?4 IS NULL OR timestamp <= ?4)
             ORDER BY timestamp
             LIMIT ?5",
        )?;
        let limit = limit
            .map(|value| i64::try_from(value).unwrap_or(i64::MAX))
            .unwrap_or(i64::MAX);
        let rows = statement.query_map(
            params![
                normalize_symbol(symbol)?,
                timeframe,
                start.map(timestamp_millis),
                end.map(timestamp_millis),
                limit,
            ],
            bar_from_row,
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("could not load bars")
    }

    pub fn latest_bar_timestamp(
        &self,
        symbol: &str,
        timeframe: &str,
    ) -> Result<Option<DateTime<Utc>>> {
        let connection = self.connection()?;
        let value: Option<i64> = connection.query_row(
            "SELECT MAX(timestamp) FROM bars
             WHERE symbol = ?1 AND timeframe = ?2
               AND NOT (
                   volume = 0 AND COALESCE(trade_count, 0) = 0
                   AND open = high AND high = low AND low = close
               )",
            params![normalize_symbol(symbol)?, timeframe],
            |row| row.get(0),
        )?;
        value
            .map(datetime_from_millis)
            .transpose()
            .map_err(Into::into)
    }

    pub fn upsert_snapshots(&self, snapshots: &[Snapshot]) -> Result<usize> {
        if snapshots.is_empty() {
            return Ok(0);
        }
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("could not begin snapshot update")?;
        {
            let mut statement = transaction.prepare_cached(
                "INSERT INTO snapshots (
                    symbol, price, previous_close, open, high, low, volume, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(symbol) DO UPDATE SET
                    price = excluded.price,
                    previous_close = excluded.previous_close,
                    open = excluded.open,
                    high = excluded.high,
                    low = excluded.low,
                    volume = excluded.volume,
                    updated_at = excluded.updated_at
                 WHERE excluded.updated_at >= snapshots.updated_at",
            )?;
            for snapshot in snapshots {
                statement.execute(params![
                    normalize_symbol(&snapshot.symbol)?,
                    snapshot.price,
                    snapshot.previous_close,
                    snapshot.open,
                    snapshot.high,
                    snapshot.low,
                    snapshot.volume,
                    timestamp_millis(snapshot.updated_at),
                ])?;
            }
        }
        transaction
            .commit()
            .context("could not commit snapshot update")?;
        Ok(snapshots.len())
    }

    pub fn snapshot(&self, symbol: &str) -> Result<Option<Snapshot>> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT symbol, price, previous_close, open, high, low, volume, updated_at
                 FROM snapshots WHERE symbol = ?1",
                [normalize_symbol(symbol)?],
                snapshot_from_row,
            )
            .optional()
            .context("could not load snapshot")
    }

    pub fn upsert_news(&self, items: &[NewsItem]) -> Result<usize> {
        if items.is_empty() {
            return Ok(0);
        }
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("could not begin news update")?;
        {
            let mut article = transaction.prepare_cached(
                "INSERT INTO news (id, headline, source, published_at, url, summary)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET
                    headline = excluded.headline,
                    source = excluded.source,
                    published_at = excluded.published_at,
                    url = excluded.url,
                    summary = excluded.summary",
            )?;
            let mut delete_symbols =
                transaction.prepare_cached("DELETE FROM news_symbols WHERE news_id = ?1")?;
            let mut insert_symbol = transaction.prepare_cached(
                "INSERT INTO news_symbols (news_id, symbol, position) VALUES (?1, ?2, ?3)",
            )?;
            for item in items {
                article.execute(params![
                    item.id,
                    item.headline,
                    item.source,
                    timestamp_millis(item.published_at),
                    item.url,
                    item.summary,
                ])?;
                delete_symbols.execute([&item.id])?;
                let mut seen = HashSet::new();
                for (position, symbol) in item.symbols.iter().enumerate() {
                    let symbol = normalize_symbol(symbol)?;
                    if seen.insert(symbol.clone()) {
                        insert_symbol.execute(params![item.id, symbol, position as i64])?;
                    }
                }
            }
        }
        transaction
            .commit()
            .context("could not commit news update")?;
        Ok(items.len())
    }

    pub fn news(&self, symbol: Option<&str>, limit: usize) -> Result<Vec<NewsItem>> {
        let connection = self.connection()?;
        let symbol = symbol.map(normalize_symbol).transpose()?;
        let mut statement = connection.prepare(
            "SELECT DISTINCT n.id, n.headline, n.source, n.published_at, n.url, n.summary
             FROM news n
             LEFT JOIN news_symbols filter_symbols ON filter_symbols.news_id = n.id
             WHERE (?1 IS NULL OR filter_symbols.symbol = ?1)
             ORDER BY n.published_at DESC, n.id
             LIMIT ?2",
        )?;
        let article_rows = statement
            .query_map(
                params![symbol, i64::try_from(limit).unwrap_or(i64::MAX)],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        datetime_from_millis(row.get(3)?)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut symbol_statement = connection.prepare(
            "SELECT symbol FROM news_symbols WHERE news_id = ?1 ORDER BY position, symbol",
        )?;
        article_rows
            .into_iter()
            .map(|(id, headline, source, published_at, url, summary)| {
                let symbols = symbol_statement
                    .query_map([&id], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()?;
                Ok(NewsItem {
                    id,
                    headline,
                    source,
                    published_at,
                    url,
                    summary,
                    symbols,
                })
            })
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("could not load news")
    }

    pub fn set_favorite(&self, symbol: &str, favorite: bool) -> Result<()> {
        let connection = self.connection()?;
        let symbol = normalize_symbol(symbol)?;
        if favorite {
            connection
                .execute(
                    "INSERT OR IGNORE INTO favorites (symbol, created_at) VALUES (?1, ?2)",
                    params![symbol, timestamp_millis(Utc::now())],
                )
                .with_context(|| format!("could not favorite {symbol}"))?;
        } else {
            connection.execute("DELETE FROM favorites WHERE symbol = ?1", [&symbol])?;
        }
        Ok(())
    }

    pub fn toggle_favorite(&self, symbol: &str) -> Result<bool> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("could not begin favorite update")?;
        let symbol = normalize_symbol(symbol)?;
        let exists = transaction
            .query_row(
                "SELECT 1 FROM favorites WHERE symbol = ?1",
                [&symbol],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if exists {
            transaction.execute("DELETE FROM favorites WHERE symbol = ?1", [&symbol])?;
        } else {
            transaction
                .execute(
                    "INSERT INTO favorites (symbol, created_at) VALUES (?1, ?2)",
                    params![symbol, timestamp_millis(Utc::now())],
                )
                .with_context(|| format!("could not favorite {symbol}"))?;
        }
        transaction.commit()?;
        Ok(!exists)
    }

    pub fn is_favorite(&self, symbol: &str) -> Result<bool> {
        let connection = self.connection()?;
        Ok(connection
            .query_row(
                "SELECT 1 FROM favorites WHERE symbol = ?1",
                [normalize_symbol(symbol)?],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn favorite_symbols(&self) -> Result<Vec<String>> {
        let connection = self.connection()?;
        let mut statement =
            connection.prepare("SELECT symbol FROM favorites ORDER BY created_at, symbol")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("could not load favorites")
    }

    pub fn favorites(&self) -> Result<Vec<Company>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {} FROM favorites f
             JOIN companies c ON c.symbol = f.symbol
             ORDER BY f.created_at, c.symbol",
            prefixed_company_columns("c")
        ))?;
        let rows = statement.query_map([], company_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("could not load favorite companies")
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Company>> {
        let term = query.trim().to_ascii_lowercase();
        if term.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let escaped = escape_like(&term);
        let contains = format!("%{escaped}%");
        let prefix = format!("{escaped}%");
        let connection = self.connection()?;
        let mut statement = connection.prepare(&format!(
            r"SELECT {COMPANY_COLUMNS} FROM companies
              WHERE lower(symbol) LIKE ?1 ESCAPE '\'
                 OR lower(name) LIKE ?1 ESCAPE '\'
              ORDER BY
                CASE
                  WHEN lower(symbol) = ?2 THEN 0
                  WHEN lower(symbol) LIKE ?3 ESCAPE '\' THEN 1
                  WHEN lower(name) LIKE ?3 ESCAPE '\' THEN 2
                  ELSE 3
                END,
                in_universe DESC,
                COALESCE(market_cap, size_proxy) IS NULL,
                COALESCE(market_cap, size_proxy) DESC,
                symbol
              LIMIT ?4"
        ))?;
        let rows = statement.query_map(
            params![
                contains,
                term,
                prefix,
                i64::try_from(limit).unwrap_or(i64::MAX)
            ],
            company_from_row,
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("could not search companies")
    }

    /// Stores a successful synchronization timestamp. Checkpoints contain no credentials.
    pub fn set_sync_checkpoint(&self, scope: &str, completed_at: DateTime<Utc>) -> Result<()> {
        self.set_sync_checkpoints(&[scope.to_owned()], completed_at)?;
        Ok(())
    }

    /// Stores multiple successful synchronization timestamps atomically.
    pub fn set_sync_checkpoints(
        &self,
        scopes: &[String],
        completed_at: DateTime<Utc>,
    ) -> Result<()> {
        if scopes.iter().any(|scope| scope.trim().is_empty()) {
            bail!("sync checkpoint scope must not be empty");
        }
        if scopes.is_empty() {
            return Ok(());
        }
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("could not begin checkpoint update")?;
        {
            let mut statement = transaction.prepare_cached(
                "INSERT INTO sync_checkpoints (scope, completed_at, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(scope) DO UPDATE SET
                    completed_at = excluded.completed_at,
                    updated_at = excluded.updated_at",
            )?;
            let updated_at = timestamp_millis(Utc::now());
            for scope in scopes {
                statement.execute(params![
                    scope.trim(),
                    timestamp_millis(completed_at),
                    updated_at
                ])?;
            }
        }
        transaction
            .commit()
            .context("could not commit checkpoint update")?;
        Ok(())
    }

    pub fn sync_checkpoint(&self, scope: &str) -> Result<Option<DateTime<Utc>>> {
        let connection = self.connection()?;
        let value: Option<i64> = connection
            .query_row(
                "SELECT completed_at FROM sync_checkpoints WHERE scope = ?1",
                [scope],
                |row| row.get(0),
            )
            .optional()?;
        value
            .map(datetime_from_millis)
            .transpose()
            .map_err(Into::into)
    }

    pub fn sync_checkpoint_scopes(&self, prefix: &str) -> Result<HashSet<String>> {
        let prefix = prefix.trim();
        if prefix.is_empty() {
            bail!("sync checkpoint prefix must not be empty");
        }
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT scope FROM sync_checkpoints
             WHERE substr(scope, 1, ?1) = ?2",
        )?;
        let rows = statement.query_map(
            params![i64::try_from(prefix.len()).unwrap_or(i64::MAX), prefix],
            |row| row.get(0),
        )?;
        rows.collect::<rusqlite::Result<HashSet<_>>>()
            .context("could not load sync checkpoint scopes")
    }

    pub fn heatmap_tiles(
        &self,
        range: DateRange,
        sort: SortMode,
        sector: Option<Sector>,
        favorites_only: bool,
        now: DateTime<Utc>,
    ) -> Result<Vec<MarketTile>> {
        self.heatmap_tiles_ordered(
            range,
            sort,
            sort.default_descending(),
            sector,
            favorites_only,
            now,
        )
    }

    pub fn heatmap_tiles_ordered(
        &self,
        range: DateRange,
        sort: SortMode,
        descending: bool,
        sector: Option<Sector>,
        favorites_only: bool,
        now: DateTime<Utc>,
    ) -> Result<Vec<MarketTile>> {
        let connection = self.connection()?;
        let companies =
            load_heatmap_companies(&connection, sector, favorites_only, now.date_naive())?;
        if companies.is_empty() {
            return Ok(Vec::new());
        }
        let favorite_symbols = load_favorite_set(&connection)?;
        let snapshots = load_snapshots(&connection)?;
        let mut timeframe_statement = connection.prepare_cached(TIMEFRAME_EXISTS_SQL)?;
        let mut metric_statement = connection.prepare_cached(PERIOD_METRIC_SQL)?;
        let mut volume_statement = connection.prepare_cached(PERIOD_VOLUME_SQL)?;
        let cutoff = range.cutoff(now);
        let mut tiles = Vec::with_capacity(companies.len());
        for company in companies {
            let timeframe = choose_timeframe(&mut timeframe_statement, range, &company.symbol)?;
            let metric = load_period_metric(
                &mut metric_statement,
                &company.symbol,
                timeframe,
                cutoff,
                now,
            )?;
            let snapshot = snapshots.get(&company.symbol);
            let period = resolve_period(range, cutoff, snapshot, metric);
            let volume = load_range_volume(
                &mut volume_statement,
                &company.symbol,
                range,
                period.source,
                snapshot,
                cutoff,
                now,
            )?;
            let updated_at = period.updated_at;
            let stale = updated_at.is_none_or(|updated| {
                now.signed_duration_since(updated).num_hours() > STALE_AFTER_HOURS
            });
            let starred = favorite_symbols.contains(&company.symbol);
            tiles.push(MarketTile {
                company,
                price: period.price,
                period_start_price: period.baseline,
                period_return: period.period_return,
                volume,
                starred,
                stale,
                updated_at,
            });
        }
        sort_and_limit_tiles(&mut tiles, sort, descending, sector, favorites_only);
        Ok(tiles)
    }

    pub fn favorite_tiles(
        &self,
        range: DateRange,
        sort: SortMode,
        now: DateTime<Utc>,
    ) -> Result<Vec<MarketTile>> {
        self.heatmap_tiles(range, sort, None, true, now)
    }

    pub fn favorite_tiles_ordered(
        &self,
        range: DateRange,
        sort: SortMode,
        descending: bool,
        now: DateTime<Utc>,
    ) -> Result<Vec<MarketTile>> {
        self.heatmap_tiles_ordered(range, sort, descending, None, true, now)
    }

    fn load_symbol_period_metric(
        &self,
        symbol: &str,
        range: DateRange,
        now: DateTime<Utc>,
    ) -> Result<(&'static str, PeriodMetric)> {
        let connection = self.connection()?;
        let mut timeframe_statement = connection.prepare_cached(TIMEFRAME_EXISTS_SQL)?;
        let timeframe = choose_timeframe(&mut timeframe_statement, range, symbol)?;
        drop(timeframe_statement);
        let mut metric_statement = connection.prepare_cached(PERIOD_METRIC_SQL)?;
        let metric = load_period_metric(
            &mut metric_statement,
            symbol,
            timeframe,
            range.cutoff(now),
            now,
        )?;
        Ok((timeframe, metric))
    }

    fn load_symbol_period_volume(
        &self,
        symbol: &str,
        range: DateRange,
        source: Option<EndpointSource>,
        snapshot: Option<&Snapshot>,
        now: DateTime<Utc>,
    ) -> Result<Option<f64>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare_cached(PERIOD_VOLUME_SQL)?;
        load_range_volume(
            &mut statement,
            symbol,
            range,
            source,
            snapshot,
            range.cutoff(now),
            now,
        )
    }

    fn load_price_history_coverage(&self, symbol: &str) -> Result<PriceHistoryCoverage> {
        let connection = self.connection()?;
        let (start, end): (Option<i64>, Option<i64>) = connection.query_row(
            "SELECT MIN(timestamp), MAX(timestamp) FROM bars
             WHERE symbol = ?1
               AND NOT (
                   volume = 0 AND COALESCE(trade_count, 0) = 0
                   AND open = high AND high = low AND low = close
               )",
            [normalize_symbol(symbol)?],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok((
            start.map(datetime_from_millis).transpose()?,
            end.map(datetime_from_millis).transpose()?,
        ))
    }

    pub fn benchmark_tiles(&self, range: DateRange, now: DateTime<Utc>) -> Result<Vec<MarketTile>> {
        let mut tiles = Vec::with_capacity(MarketBenchmark::ALL.len());
        for benchmark in MarketBenchmark::ALL {
            let Some(company) = self.company(benchmark.symbol)? else {
                continue;
            };
            let (_, metric) = self.load_symbol_period_metric(benchmark.symbol, range, now)?;
            let snapshot = self.snapshot(benchmark.symbol)?;
            let period = resolve_period(range, range.cutoff(now), snapshot.as_ref(), metric);
            let updated_at = period.updated_at;
            let volume = self.load_symbol_period_volume(
                benchmark.symbol,
                range,
                period.source,
                snapshot.as_ref(),
                now,
            )?;
            tiles.push(MarketTile {
                price: period.price,
                period_start_price: period.baseline,
                period_return: period.period_return,
                volume,
                starred: self.is_favorite(benchmark.symbol)?,
                stale: updated_at.is_none_or(|updated| {
                    now.signed_duration_since(updated).num_hours() > STALE_AFTER_HOURS
                }),
                updated_at,
                company,
            });
        }
        Ok(tiles)
    }

    pub fn ticker_detail(
        &self,
        symbol: &str,
        range: DateRange,
        now: DateTime<Utc>,
        news_limit: usize,
    ) -> Result<Option<TickerDetail>> {
        let Some(company) = self.company(symbol)? else {
            return Ok(None);
        };
        let (timeframe, metric) = self.load_symbol_period_metric(&company.symbol, range, now)?;
        let bars = self
            .bars(
                &company.symbol,
                Some(timeframe),
                Some(range.detail_history_cutoff(now)),
                Some(now),
                None,
            )?
            .into_iter()
            .filter(Bar::is_price_observation)
            .collect();
        let (history_start_at, history_end_at) =
            self.load_price_history_coverage(&company.symbol)?;
        let range_start_at = if range == DateRange::All {
            history_start_at.unwrap_or_else(|| range.cutoff(now))
        } else {
            range.cutoff(now)
        };
        let snapshot = self.snapshot(&company.symbol)?;
        let period = resolve_period(range, range.cutoff(now), snapshot.as_ref(), metric);
        let own_tiles = company
            .sector
            .map(|sector| self.heatmap_tiles(range, SortMode::Gainers, Some(sector), false, now))
            .transpose()?
            .unwrap_or_default();
        let returns = own_tiles
            .iter()
            .filter_map(|tile| tile.period_return)
            .collect::<Vec<_>>();
        let sector_return =
            (!returns.is_empty()).then(|| returns.iter().sum::<f64>() / returns.len() as f64);
        let sector_rank = own_tiles
            .iter()
            .position(|tile| tile.company.symbol == company.symbol)
            .map(|index| index + 1);
        Ok(Some(TickerDetail {
            news: self.news(Some(&company.symbol), news_limit)?,
            starred: self.is_favorite(&company.symbol)?,
            company,
            snapshot,
            bars,
            history_start_at,
            history_end_at,
            range_start_at,
            range_end_at: now,
            period_start_price: period.baseline,
            period_start_at: period.baseline_at,
            period_end_price: period.price,
            period_end_at: period.updated_at,
            period_return: period.period_return,
            sector_return,
            sector_rank,
        }))
    }

    pub fn counts(&self) -> Result<StorageCounts> {
        let connection = self.connection()?;
        Ok(StorageCounts {
            companies: table_count(&connection, "companies")?,
            memberships: table_count(&connection, "sector_memberships")?,
            bars: table_count(&connection, "bars")?,
            snapshots: table_count(&connection, "snapshots")?,
            news: table_count(&connection, "news")?,
            favorites: table_count(&connection, "favorites")?,
            checkpoints: table_count(&connection, "sync_checkpoints")?,
        })
    }

    /// Removes simulated records before a legacy shared cache is used in live mode.
    ///
    /// Demo and live bars use different provider timestamps, so retaining both
    /// would make one apparent series alternate between unrelated prices.
    pub fn purge_demo_data_for_live(&self) -> Result<bool> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("could not begin demo cache cleanup")?;
        let has_demo_data: bool = transaction.query_row(
            "SELECT
                EXISTS(
                    SELECT 1 FROM sync_checkpoints
                    WHERE scope = 'demo' OR scope LIKE 'demo:%'
                )
                OR EXISTS(SELECT 1 FROM bars WHERE source = 'demo')
                OR EXISTS(
                    SELECT 1 FROM news
                    WHERE id LIKE 'demo-%' OR source LIKE 'SIMULATED%'
                )",
            [],
            |row| row.get(0),
        )?;
        if !has_demo_data {
            transaction
                .commit()
                .context("could not finish demo cache inspection")?;
            return Ok(false);
        }

        transaction.execute(
            "DELETE FROM sector_memberships
             WHERE symbol IN (
                 SELECT symbol FROM companies
                 WHERE raw_sector LIKE '%SIMULATED DEMO%'
             )",
            [],
        )?;
        transaction.execute("DELETE FROM bars WHERE source = 'demo'", [])?;
        transaction.execute(
            "DELETE FROM news
             WHERE id LIKE 'demo-%' OR source LIKE 'SIMULATED%'",
            [],
        )?;
        // Snapshots predate source tracking, so their provenance is ambiguous.
        transaction.execute("DELETE FROM snapshots", [])?;
        transaction.execute(
            "UPDATE companies
             SET market_cap = NULL,
                 shares_outstanding = NULL,
                 shares_source = NULL,
                 shares_as_of = NULL,
                 shares_method = NULL,
                 shares_confidence = NULL,
                 in_universe = 0,
                 retained = 1
             WHERE raw_sector LIKE '%SIMULATED DEMO%'",
            [],
        )?;
        transaction.execute(
            "DELETE FROM sync_checkpoints
             WHERE scope = 'snapshots'
                OR scope = 'demo'
                OR scope LIKE 'demo:%'",
            [],
        )?;
        transaction
            .commit()
            .context("could not commit demo cache cleanup")?;
        Ok(true)
    }

    /// Clears the selected cache before deterministic demo data is regenerated.
    pub fn reset_demo_data(&self) -> Result<()> {
        self.reset()
    }

    pub fn reset(&self) -> Result<()> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("could not begin cache reset")?;
        transaction.execute("DELETE FROM favorites", [])?;
        transaction.execute("DELETE FROM news", [])?;
        transaction.execute("DELETE FROM snapshots", [])?;
        transaction.execute("DELETE FROM bars", [])?;
        transaction.execute("DELETE FROM sector_memberships", [])?;
        transaction.execute("DELETE FROM companies", [])?;
        transaction.execute("DELETE FROM sync_checkpoints", [])?;
        transaction.execute("DELETE FROM cache_context", [])?;
        transaction.commit().context("could not commit cache reset")
    }
}

impl From<&CacheIdentity> for StoredCacheIdentity {
    fn from(identity: &CacheIdentity) -> Self {
        Self {
            namespace: identity.namespace.to_string(),
            market_id: identity.market.id.to_string(),
            symbol_namespace: identity.market.symbol_namespace.to_string(),
            currency: identity.market.currency.to_string(),
            timezone: identity.market.timezone.to_string(),
            regular_open: identity.market.regular_open.format("%H:%M:%S").to_string(),
            regular_close: identity.market.regular_close.format("%H:%M:%S").to_string(),
        }
    }
}

impl StoredCacheIdentity {
    fn into_domain(self) -> Result<CacheIdentity> {
        let timezone = self
            .timezone
            .parse()
            .with_context(|| format!("invalid cached market timezone {:?}", self.timezone))?;
        let regular_open =
            NaiveTime::parse_from_str(&self.regular_open, "%H:%M:%S").with_context(|| {
                format!(
                    "invalid cached regular-session open {:?}",
                    self.regular_open
                )
            })?;
        let regular_close = NaiveTime::parse_from_str(&self.regular_close, "%H:%M:%S")
            .with_context(|| {
                format!(
                    "invalid cached regular-session close {:?}",
                    self.regular_close
                )
            })?;
        if regular_close <= regular_open {
            bail!("cached regular-session close must be later than its open");
        }
        Ok(CacheIdentity::new(
            self.namespace,
            MarketContext {
                id: self.market_id.into(),
                symbol_namespace: self.symbol_namespace.into(),
                currency: self.currency.into(),
                timezone,
                regular_open,
                regular_close,
            },
        ))
    }
}

fn validate_cache_identity(identity: &CacheIdentity) -> Result<()> {
    if identity.namespace.trim().is_empty()
        || identity.market.id.trim().is_empty()
        || identity.market.symbol_namespace.trim().is_empty()
        || identity.market.currency.trim().is_empty()
    {
        bail!("cache identity fields must not be empty");
    }
    if identity.market.regular_close <= identity.market.regular_open {
        bail!("regular-session close must be later than its open");
    }
    Ok(())
}

fn load_stored_cache_identity(connection: &Connection) -> Result<Option<StoredCacheIdentity>> {
    connection
        .query_row(
            "SELECT
                namespace, market_id, symbol_namespace, currency, timezone,
                regular_open, regular_close
             FROM cache_context
             WHERE singleton = 1",
            [],
            |row| {
                Ok(StoredCacheIdentity {
                    namespace: row.get(0)?,
                    market_id: row.get(1)?,
                    symbol_namespace: row.get(2)?,
                    currency: row.get(3)?,
                    timezone: row.get(4)?,
                    regular_open: row.get(5)?,
                    regular_close: row.get(6)?,
                })
            },
        )
        .optional()
        .context("could not load cache identity")
}

fn store_cache_identity(
    transaction: &Transaction<'_>,
    identity: &StoredCacheIdentity,
) -> Result<()> {
    transaction
        .execute(
            "INSERT INTO cache_context (
                singleton, namespace, market_id, symbol_namespace, currency,
                timezone, regular_open, regular_close, updated_at
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(singleton) DO UPDATE SET
                namespace = excluded.namespace,
                market_id = excluded.market_id,
                symbol_namespace = excluded.symbol_namespace,
                currency = excluded.currency,
                timezone = excluded.timezone,
                regular_open = excluded.regular_open,
                regular_close = excluded.regular_close,
                updated_at = excluded.updated_at",
            params![
                identity.namespace,
                identity.market_id,
                identity.symbol_namespace,
                identity.currency,
                identity.timezone,
                identity.regular_open,
                identity.regular_close,
                timestamp_millis(Utc::now()),
            ],
        )
        .context("could not store cache identity")?;
    Ok(())
}

fn clear_incompatible_live_rows(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute("DELETE FROM news", [])?;
    transaction.execute("DELETE FROM snapshots", [])?;
    transaction.execute("DELETE FROM bars", [])?;
    transaction.execute("DELETE FROM sector_memberships", [])?;
    transaction.execute("DELETE FROM sync_checkpoints", [])?;
    transaction.execute(
        "DELETE FROM companies
         WHERE symbol NOT IN (SELECT symbol FROM favorites)",
        [],
    )?;
    transaction.execute(
        "UPDATE companies
         SET name = symbol,
             sector = NULL,
             raw_sector = NULL,
             exchange = '',
             industry = '',
             market_cap = NULL,
             size_proxy = NULL,
             size_proxy_source = NULL,
             size_proxy_as_of = NULL,
             size_proxy_confidence = NULL,
             shares_outstanding = NULL,
             shares_source = NULL,
             shares_as_of = NULL,
             shares_method = NULL,
             shares_confidence = NULL,
             rank = NULL,
             description = '',
             in_universe = 0,
             retained = 1,
             updated_at = ?1",
        [timestamp_millis(Utc::now())],
    )?;
    Ok(())
}

fn upsert_company(
    transaction: &Transaction<'_>,
    company: &Company,
    force_in_universe: Option<bool>,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO companies (
            symbol, name, sector, raw_sector, exchange, industry, market_cap,
            size_proxy, size_proxy_source, size_proxy_as_of, size_proxy_confidence,
            shares_outstanding, shares_source, shares_as_of, shares_method, shares_confidence,
            rank, description, in_universe, retained, updated_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
            ?15, ?16, ?17, ?18, ?19, ?20, ?21
         )
         ON CONFLICT(symbol) DO UPDATE SET
            name = excluded.name,
            sector = excluded.sector,
            raw_sector = excluded.raw_sector,
            exchange = excluded.exchange,
            industry = excluded.industry,
            market_cap = excluded.market_cap,
            size_proxy = excluded.size_proxy,
            size_proxy_source = excluded.size_proxy_source,
            size_proxy_as_of = excluded.size_proxy_as_of,
            size_proxy_confidence = excluded.size_proxy_confidence,
            shares_outstanding = excluded.shares_outstanding,
            shares_source = excluded.shares_source,
            shares_as_of = excluded.shares_as_of,
            shares_method = excluded.shares_method,
            shares_confidence = excluded.shares_confidence,
            rank = excluded.rank,
            description = excluded.description,
            in_universe = excluded.in_universe,
            retained = excluded.retained,
            updated_at = excluded.updated_at",
        params![
            normalize_symbol(&company.symbol)?,
            company.name,
            company.sector.map(sector_key),
            company.raw_sector,
            company.exchange,
            company.industry,
            company.market_cap,
            company.size_proxy,
            company.size_proxy_source,
            company.size_proxy_as_of.map(|date| date.to_string()),
            company.size_proxy_confidence,
            company.shares_outstanding,
            company.shares_source,
            company.shares_as_of.map(|date| date.to_string()),
            company.shares_method,
            company.shares_confidence,
            company.rank.map(i64::from),
            company.description,
            force_in_universe.unwrap_or(company.in_universe),
            company.retained,
            timestamp_millis(company.updated_at),
        ],
    )?;
    Ok(())
}

fn replace_sector_memberships(
    transaction: &Transaction<'_>,
    as_of: NaiveDate,
    sector: Sector,
    companies: &[&Company],
) -> Result<()> {
    transaction.execute(
        "DELETE FROM sector_memberships WHERE as_of_date = ?1 AND sector = ?2",
        params![as_of.to_string(), sector_key(sector)],
    )?;
    insert_sector_memberships(transaction, as_of, sector, companies)
}

fn insert_sector_memberships(
    transaction: &Transaction<'_>,
    as_of: NaiveDate,
    sector: Sector,
    companies: &[&Company],
) -> Result<()> {
    let mut statement = transaction.prepare_cached(
        "INSERT INTO sector_memberships (
            as_of_date, sector, symbol, rank, market_cap, size_proxy
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for (position, company) in companies.iter().enumerate() {
        statement.execute(params![
            as_of.to_string(),
            sector_key(sector),
            normalize_symbol(&company.symbol)?,
            i64::try_from(position + 1).unwrap_or(i64::MAX),
            company.market_cap,
            company.size_proxy,
        ])?;
    }
    Ok(())
}

fn selected_members(companies: &[Company], limit: usize) -> Vec<&Company> {
    selected_members_from_refs(&companies.iter().collect::<Vec<_>>(), limit)
}

fn selected_members_from_refs<'a>(companies: &[&'a Company], limit: usize) -> Vec<&'a Company> {
    let mut selected = companies.to_vec();
    selected.sort_by(|left, right| {
        compare_optional_f64(
            screened_company_size(left),
            screened_company_size(right),
            true,
        )
        .then_with(|| left.rank.is_none().cmp(&right.rank.is_none()))
        .then_with(|| left.rank.cmp(&right.rank))
        .then_with(|| left.symbol.cmp(&right.symbol))
    });
    selected.truncate(limit);
    selected
}

fn load_heatmap_companies(
    connection: &Connection,
    sector: Option<Sector>,
    favorites_only: bool,
    as_of: NaiveDate,
) -> Result<Vec<Company>> {
    if favorites_only {
        let mut statement = connection.prepare(&format!(
            "SELECT {} FROM favorites f
             JOIN companies c ON c.symbol = f.symbol
             WHERE (?1 IS NULL OR c.sector = ?1)",
            prefixed_company_columns("c")
        ))?;
        let rows = statement.query_map([sector.map(sector_key)], company_from_row)?;
        return rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("could not load favorite heatmap companies");
    }

    let mut statement = connection.prepare(
        "WITH latest AS (
            SELECT sector, MAX(as_of_date) AS as_of_date
            FROM sector_memberships
            WHERE as_of_date <= ?1
            GROUP BY sector
         )
         SELECT c.symbol, c.name, memberships.sector, c.raw_sector, c.exchange, c.industry,
                memberships.market_cap, memberships.size_proxy, c.size_proxy_source,
                c.size_proxy_as_of, c.size_proxy_confidence,
                c.shares_outstanding, c.shares_source, c.shares_as_of,
                c.shares_method, c.shares_confidence,
                memberships.rank, c.description, c.in_universe, c.retained, c.updated_at
         FROM latest
         JOIN sector_memberships memberships
           ON memberships.sector = latest.sector
          AND memberships.as_of_date = latest.as_of_date
         JOIN companies c ON c.symbol = memberships.symbol
         WHERE (?2 IS NULL OR memberships.sector = ?2)
         ORDER BY memberships.sector, memberships.rank, c.symbol",
    )?;
    let rows = statement
        .query_map(
            params![as_of.to_string(), sector.map(sector_key)],
            company_from_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !rows.is_empty() {
        return Ok(rows);
    }

    let mut fallback = connection.prepare(&format!(
        "SELECT {COMPANY_COLUMNS} FROM companies
         WHERE in_universe = 1 AND (?1 IS NULL OR sector = ?1)"
    ))?;
    fallback
        .query_map([sector.map(sector_key)], company_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("could not load fallback heatmap companies")
}

fn load_favorite_set(connection: &Connection) -> Result<HashSet<String>> {
    let mut statement = connection.prepare("SELECT symbol FROM favorites")?;
    statement
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<HashSet<_>>>()
        .context("could not load favorite symbols")
}

fn load_snapshots(connection: &Connection) -> Result<HashMap<String, Snapshot>> {
    let mut statement = connection.prepare(
        "SELECT symbol, price, previous_close, open, high, low, volume, updated_at FROM snapshots",
    )?;
    statement
        .query_map([], snapshot_from_row)?
        .map(|result| result.map(|snapshot| (snapshot.symbol.clone(), snapshot)))
        .collect::<rusqlite::Result<HashMap<_, _>>>()
        .context("could not load snapshots")
}

fn timeframe_candidates(range: DateRange) -> &'static [&'static str] {
    match range {
        DateRange::Day => &["5Min", "15Min", "1Hour", "1Day"],
        DateRange::Week => &["1Hour", "1Day", "15Min", "5Min", "1Week"],
        DateRange::Month => &["1Hour", "1Day", "1Week"],
        DateRange::ThreeMonths | DateRange::SixMonths => &["1Day", "1Hour", "1Week"],
        DateRange::Year | DateRange::TwoYears => &["1Day", "1Week", "1Hour"],
        DateRange::FiveYears | DateRange::TenYears | DateRange::All => &["1Week", "1Day"],
    }
}

fn choose_timeframe(
    statement: &mut rusqlite::CachedStatement<'_>,
    range: DateRange,
    symbol: &str,
) -> Result<&'static str> {
    for candidate in timeframe_candidates(range) {
        let exists =
            statement.query_row(params![symbol, candidate], |row| row.get::<_, bool>(0))?;
        if exists {
            return Ok(candidate);
        }
    }
    Ok(range.preferred_timeframe())
}

fn load_period_metric(
    statement: &mut rusqlite::CachedStatement<'_>,
    symbol: &str,
    timeframe: &str,
    cutoff: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<PeriodMetric> {
    let (baseline, baseline_at, close, timestamp): PeriodMetricRow = statement.query_row(
        params![
            symbol,
            timeframe,
            timestamp_millis(cutoff),
            timestamp_millis(now)
        ],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    Ok(PeriodMetric {
        baseline,
        baseline_at: baseline_at.map(datetime_from_millis).transpose()?,
        close,
        updated_at: timestamp.map(datetime_from_millis).transpose()?,
    })
}

fn load_range_volume(
    statement: &mut rusqlite::CachedStatement<'_>,
    symbol: &str,
    range: DateRange,
    source: Option<EndpointSource>,
    snapshot: Option<&Snapshot>,
    cutoff: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<Option<f64>> {
    if range == DateRange::Day
        && source == Some(EndpointSource::Snapshot)
        && let Some(volume) = snapshot
            .and_then(|value| value.volume)
            .filter(|volume| volume.is_finite() && *volume >= 0.0)
    {
        return Ok(Some(volume));
    }

    for timeframe in volume_timeframe_candidates(range) {
        let volume = statement
            .query_row(
                params![
                    symbol,
                    timeframe,
                    timestamp_millis(cutoff),
                    timestamp_millis(now)
                ],
                |row| row.get::<_, Option<f64>>(0),
            )?
            .filter(|volume| volume.is_finite() && *volume >= 0.0);
        if volume.is_some() {
            return Ok(volume);
        }
    }
    Ok(None)
}

fn volume_timeframe_candidates(range: DateRange) -> &'static [&'static str] {
    match range {
        DateRange::Day => &["1Day", "1Hour", "15Min", "5Min"],
        DateRange::FiveYears | DateRange::TenYears | DateRange::All => {
            &["1Week", "1Day", "1Hour", "15Min", "5Min"]
        }
        _ => &["1Day", "1Hour", "15Min", "5Min", "1Week"],
    }
}

fn resolve_period(
    range: DateRange,
    cutoff: DateTime<Utc>,
    snapshot: Option<&Snapshot>,
    metric: PeriodMetric,
) -> ResolvedPeriod {
    let snapshot_endpoint = snapshot.and_then(|value| {
        value
            .price
            .filter(|price| valid_stock_price(*price))
            .map(|price| (price, value.updated_at))
    });
    let bar_endpoint = metric
        .close
        .filter(|price| valid_stock_price(*price))
        .map(|price| (price, metric.updated_at));
    let (price, updated_at, source) = match (snapshot_endpoint, bar_endpoint) {
        (Some((_snapshot_price, snapshot_at)), Some((bar_price, Some(bar_at))))
            if bar_at > snapshot_at =>
        {
            (Some(bar_price), Some(bar_at), Some(EndpointSource::Bar))
        }
        (Some((snapshot_price, snapshot_at)), Some(_)) => (
            Some(snapshot_price),
            Some(snapshot_at),
            Some(EndpointSource::Snapshot),
        ),
        (Some((snapshot_price, snapshot_at)), None) => (
            Some(snapshot_price),
            Some(snapshot_at),
            Some(EndpointSource::Snapshot),
        ),
        (None, Some((bar_price, bar_at))) => (Some(bar_price), bar_at, Some(EndpointSource::Bar)),
        (None, None) => (None, None, None),
    };
    let metric_baseline = metric.baseline.filter(|value| valid_stock_price(*value));
    let snapshot_day_baseline = (range == DateRange::Day
        && source == Some(EndpointSource::Snapshot))
    .then(|| {
        snapshot
            .and_then(|value| value.previous_close)
            .filter(|value| valid_stock_price(*value))
    })
    .flatten();
    let (baseline, baseline_at) = if let Some(baseline) = snapshot_day_baseline {
        (Some(baseline), metric.baseline_at.or(Some(cutoff)))
    } else {
        (metric_baseline, metric.baseline_at)
    };
    let period_return = price
        .zip(baseline)
        .map(|(latest, baseline)| latest / baseline - 1.0);
    ResolvedPeriod {
        price,
        baseline,
        baseline_at,
        period_return,
        updated_at,
        source,
    }
}

fn valid_stock_price(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn sort_and_limit_tiles(
    tiles: &mut Vec<MarketTile>,
    sort: SortMode,
    descending: bool,
    selected_sector: Option<Sector>,
    favorites_only: bool,
) {
    let compare = |left: &MarketTile, right: &MarketTile| {
        compare_tiles(left, right, sort, descending)
            .then_with(|| left.company.symbol.cmp(&right.company.symbol))
    };
    if selected_sector.is_some() || favorites_only {
        tiles.sort_by(compare);
        if !favorites_only {
            tiles.truncate(MAX_MEMBERS_PER_SECTOR);
        }
        return;
    }

    let mut grouped: HashMap<Sector, Vec<MarketTile>> = HashMap::new();
    let mut unclassified = Vec::new();
    for tile in std::mem::take(tiles) {
        if let Some(sector) = tile.company.sector {
            grouped.entry(sector).or_default().push(tile);
        } else {
            unclassified.push(tile);
        }
    }
    for sector in Sector::ALL {
        if let Some(mut sector_tiles) = grouped.remove(&sector) {
            sector_tiles.sort_by(compare);
            sector_tiles.truncate(MAX_MEMBERS_PER_SECTOR);
            tiles.extend(sector_tiles);
        }
    }
    unclassified.sort_by(compare);
    unclassified.truncate(MAX_MEMBERS_PER_SECTOR);
    tiles.extend(unclassified);
}

fn compare_tiles(
    left: &MarketTile,
    right: &MarketTile,
    sort: SortMode,
    descending: bool,
) -> Ordering {
    match sort {
        SortMode::MarketCap => compare_optional_f64(
            screened_company_size(&left.company),
            screened_company_size(&right.company),
            descending,
        )
        .then_with(|| {
            if descending {
                left.company.rank.cmp(&right.company.rank)
            } else {
                right.company.rank.cmp(&left.company.rank)
            }
        }),
        SortMode::Gainers => {
            compare_optional_f64(left.period_return, right.period_return, descending)
        }
        SortMode::Volume => compare_optional_f64(left.volume, right.volume, descending),
        SortMode::Alphabetical if descending => right.company.symbol.cmp(&left.company.symbol),
        SortMode::Alphabetical => left.company.symbol.cmp(&right.company.symbol),
    }
}

fn compare_optional_f64(left: Option<f64>, right: Option<f64>, descending: bool) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) if descending => right.total_cmp(&left),
        (Some(left), Some(right)) => left.total_cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn screened_company_size(company: &Company) -> Option<f64> {
    company
        .market_cap
        .filter(|value| value.is_finite() && *value > 0.0)
        .or_else(|| {
            company
                .size_proxy
                .filter(|value| value.is_finite() && *value > 0.0)
        })
}

fn table_count(connection: &Connection, table: &str) -> Result<usize> {
    let count: i64 = connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })?;
    usize::try_from(count).context("SQLite returned a negative row count")
}

fn company_from_row(row: &Row<'_>) -> rusqlite::Result<Company> {
    let rank = row
        .get::<_, Option<i64>>(16)?
        .map(|value| {
            u16::try_from(value).map_err(|error| conversion_error(16, Type::Integer, error))
        })
        .transpose()?;
    Ok(Company {
        symbol: row.get(0)?,
        name: row.get(1)?,
        sector: row
            .get::<_, Option<String>>(2)?
            .map(|value| {
                parse_sector(&value).map_err(|error| conversion_error(2, Type::Text, error))
            })
            .transpose()?,
        raw_sector: row.get(3)?,
        exchange: row.get(4)?,
        industry: row.get(5)?,
        market_cap: row.get(6)?,
        size_proxy: row.get(7)?,
        size_proxy_source: row.get(8)?,
        size_proxy_as_of: optional_date_from_row(row, 9)?,
        size_proxy_confidence: row.get(10)?,
        shares_outstanding: row.get(11)?,
        shares_source: row.get(12)?,
        shares_as_of: optional_date_from_row(row, 13)?,
        shares_method: row.get(14)?,
        shares_confidence: row.get(15)?,
        rank,
        description: row.get(17)?,
        in_universe: row.get(18)?,
        retained: row.get(19)?,
        updated_at: datetime_from_millis(row.get(20)?)?,
    })
}

fn optional_date_from_row(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<NaiveDate>> {
    row.get::<_, Option<String>>(index)?
        .map(|value| {
            NaiveDate::parse_from_str(&value, "%Y-%m-%d")
                .map_err(|error| conversion_error(index, Type::Text, error))
        })
        .transpose()
}

fn bar_from_row(row: &Row<'_>) -> rusqlite::Result<Bar> {
    let trade_count = row
        .get::<_, Option<i64>>(8)?
        .map(|value| {
            u64::try_from(value).map_err(|error| conversion_error(8, Type::Integer, error))
        })
        .transpose()?;
    Ok(Bar {
        symbol: row.get(0)?,
        timeframe: row.get(1)?,
        timestamp: datetime_from_millis(row.get(2)?)?,
        open: row.get(3)?,
        high: row.get(4)?,
        low: row.get(5)?,
        close: row.get(6)?,
        volume: row.get(7)?,
        trade_count,
        vwap: row.get(9)?,
        source: row.get(10)?,
    })
}

fn snapshot_from_row(row: &Row<'_>) -> rusqlite::Result<Snapshot> {
    Ok(Snapshot {
        symbol: row.get(0)?,
        price: row.get(1)?,
        market_cap: None,
        previous_close: row.get(2)?,
        open: row.get(3)?,
        high: row.get(4)?,
        low: row.get(5)?,
        volume: row.get(6)?,
        updated_at: datetime_from_millis(row.get(7)?)?,
    })
}

fn datetime_from_millis(value: i64) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::from_timestamp_millis(value).ok_or_else(|| {
        conversion_error(
            0,
            Type::Integer,
            std::io::Error::new(std::io::ErrorKind::InvalidData, "timestamp is out of range"),
        )
    })
}

fn conversion_error(
    index: usize,
    value_type: Type,
    error: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, value_type, Box::new(error))
}

fn timestamp_millis(value: DateTime<Utc>) -> i64 {
    value.timestamp_millis()
}

fn optional_u64_to_i64(value: Option<u64>) -> Result<Option<i64>> {
    value
        .map(|value| i64::try_from(value).context("trade count exceeds SQLite integer range"))
        .transpose()
}

fn normalize_symbol(symbol: &str) -> Result<String> {
    let symbol = symbol.trim().to_ascii_uppercase();
    if symbol.is_empty() {
        bail!("stock symbol must not be empty");
    }
    Ok(symbol)
}

const fn sector_key(sector: Sector) -> &'static str {
    match sector {
        Sector::Consumer => "consumer",
        Sector::Services => "services",
        Sector::Healthcare => "healthcare",
        Sector::Energy => "energy",
        Sector::Technology => "technology",
        Sector::Financial => "financial",
        Sector::Industrial => "industrial",
        Sector::Materials => "materials",
        Sector::Utilities => "utilities",
    }
}

fn parse_sector(value: &str) -> std::io::Result<Sector> {
    match value {
        "consumer" => Ok(Sector::Consumer),
        "services" => Ok(Sector::Services),
        "healthcare" => Ok(Sector::Healthcare),
        "energy" => Ok(Sector::Energy),
        "technology" => Ok(Sector::Technology),
        "financial" => Ok(Sector::Financial),
        "industrial" => Ok(Sector::Industrial),
        "materials" => Ok(Sector::Materials),
        "utilities" => Ok(Sector::Utilities),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("unknown stored sector {value:?}"),
        )),
    }
}

fn prefixed_company_columns(prefix: &str) -> String {
    COMPANY_COLUMNS
        .split(',')
        .map(str::trim)
        .map(|column| format!("{prefix}.{column}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, thread};

    use chrono::{TimeZone, Utc};
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::*;

    fn instant(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, day, 20, 0, 0)
            .single()
            .expect("valid test timestamp")
    }

    fn company(
        symbol: &str,
        name: &str,
        sector: Sector,
        market_cap: f64,
        rank: Option<u16>,
        now: DateTime<Utc>,
    ) -> Company {
        Company {
            symbol: symbol.to_owned(),
            name: name.to_owned(),
            sector: Some(sector),
            raw_sector: Some(sector.label().to_owned()),
            exchange: "NASDAQ".to_owned(),
            industry: "Software".to_owned(),
            market_cap: Some(market_cap),
            size_proxy: None,
            size_proxy_source: None,
            size_proxy_as_of: None,
            size_proxy_confidence: None,
            shares_outstanding: Some(1_000_000.0),
            shares_source: Some("test".to_owned()),
            shares_as_of: Some(now.date_naive()),
            shares_method: Some("test".to_owned()),
            shares_confidence: Some("high".to_owned()),
            rank,
            description: format!("{name} description"),
            in_universe: true,
            retained: false,
            updated_at: now,
        }
    }

    fn bar(symbol: &str, timestamp: DateTime<Utc>, close: f64, volume: f64) -> Bar {
        Bar {
            symbol: symbol.to_owned(),
            timeframe: "1Day".to_owned(),
            timestamp,
            open: close - 1.0,
            high: close + 1.0,
            low: close - 2.0,
            close,
            volume,
            trade_count: Some(42),
            vwap: Some(close - 0.25),
            source: "test".to_owned(),
        }
    }

    fn no_trade_bar(
        symbol: &str,
        timeframe: &str,
        timestamp: DateTime<Utc>,
        close: f64,
        trade_count: Option<u64>,
    ) -> Bar {
        Bar {
            symbol: symbol.to_owned(),
            timeframe: timeframe.to_owned(),
            timestamp,
            open: close,
            high: close,
            low: close,
            close,
            volume: 0.0,
            trade_count,
            vwap: None,
            source: "test".to_owned(),
        }
    }

    fn snapshot(
        symbol: &str,
        price: f64,
        previous_close: f64,
        volume: f64,
        now: DateTime<Utc>,
    ) -> Snapshot {
        Snapshot {
            symbol: symbol.to_owned(),
            price: Some(price),
            market_cap: None,
            previous_close: Some(previous_close),
            open: Some(previous_close),
            high: Some(price.max(previous_close) + 1.0),
            low: Some(price.min(previous_close) - 1.0),
            volume: Some(volume),
            updated_at: now,
        }
    }

    #[test]
    fn migrates_wal_and_preserves_dated_memberships() -> Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("market.sqlite3");
        let storage = Storage::open(&path)?;
        assert_eq!(storage.schema_version()?, 3);
        assert_eq!(storage.journal_mode()?.to_ascii_lowercase(), "wal");

        let now = instant(13);
        let apple = company("aapl", "Apple", Sector::Technology, 3_000.0, Some(1), now);
        let microsoft = company(
            "MSFT",
            "Microsoft",
            Sector::Technology,
            2_800.0,
            Some(2),
            now,
        );
        let nvidia = company("NVDA", "Nvidia", Sector::Technology, 2_600.0, None, now);
        let june = NaiveDate::from_ymd_opt(2026, 6, 30).unwrap();
        let july = NaiveDate::from_ymd_opt(2026, 7, 12).unwrap();
        storage.replace_memberships(june, Sector::Technology, &[apple, microsoft.clone()])?;
        storage.replace_memberships(july, Sector::Technology, &[microsoft, nvidia])?;
        let mut updated_apple = storage.company("AAPL")?.expect("Apple remains cached");
        updated_apple.market_cap = Some(9_999.0);
        updated_apple.size_proxy = Some(8_888.0);
        storage.upsert_companies(&[updated_apple])?;

        assert_eq!(
            storage.latest_membership_date(Some(Sector::Technology))?,
            Some(july)
        );
        let june_members = storage.memberships(Sector::Technology, Some(june))?;
        assert_eq!(
            june_members
                .iter()
                .map(|value| value.symbol.as_str())
                .collect::<Vec<_>>(),
            vec!["AAPL", "MSFT"]
        );
        assert_eq!(june_members[0].market_cap, Some(3_000.0));
        assert_eq!(june_members[0].size_proxy, None);
        let current = storage.memberships(Sector::Technology, None)?;
        assert_eq!(
            current
                .iter()
                .map(|value| value.symbol.as_str())
                .collect::<Vec<_>>(),
            ["MSFT", "NVDA"]
        );
        assert_eq!(current[1].rank, Some(2));

        drop(storage);
        let reopened = Storage::open(&path)?;
        assert_eq!(reopened.company("msft")?.unwrap().name, "Microsoft");
        assert_eq!(reopened.counts()?.memberships, 4);
        Ok(())
    }

    #[test]
    fn migrates_version_one_databases_without_discarding_company_rows() -> Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("market.sqlite3");
        let connection = Connection::open(&path)?;
        connection.execute_batch(&format!("{MIGRATION_1}\nPRAGMA user_version = 1;"))?;
        connection.execute(
            "INSERT INTO companies (
                symbol, name, sector, exchange, updated_at
             ) VALUES ('LEGACY', 'Legacy Company', 'technology', 'NASDAQ', ?1)",
            [timestamp_millis(instant(12))],
        )?;
        connection.execute(
            "INSERT INTO favorites (symbol, created_at) VALUES ('LEGACY', ?1)",
            [timestamp_millis(instant(12))],
        )?;
        connection.execute(
            "INSERT INTO bars (
                symbol, timeframe, timestamp, open, high, low, close, volume
             ) VALUES ('LEGACY', '1Day', ?1, 9.0, 11.0, 8.0, 10.0, 100.0)",
            [timestamp_millis(instant(12))],
        )?;
        connection.execute(
            "INSERT INTO sector_memberships (
                as_of_date, sector, symbol, rank, market_cap
             ) VALUES ('2026-07-23', 'technology', 'LEGACY', 1, 1000.0)",
            [],
        )?;
        drop(connection);

        let storage = Storage::open(&path)?;
        assert_eq!(storage.schema_version()?, 3);
        let legacy = storage.company("LEGACY")?.expect("legacy company survives");
        assert_eq!(legacy.name, "Legacy Company");
        assert_eq!(legacy.size_proxy, None);
        assert_eq!(legacy.shares_source, None);
        assert!(storage.is_favorite("LEGACY")?);
        assert_eq!(
            storage
                .bars("LEGACY", Some("1Day"), None, None, None)?
                .len(),
            1
        );
        assert_eq!(
            storage
                .memberships(Sector::Technology, None)?
                .into_iter()
                .map(|company| company.symbol)
                .collect::<Vec<_>>(),
            ["LEGACY"]
        );
        Ok(())
    }

    #[test]
    fn rejects_a_database_from_a_newer_schema() -> Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("future.sqlite3");
        let connection = Connection::open(&path)?;
        connection.execute_batch("PRAGMA user_version = 4;")?;
        drop(connection);

        let error = Storage::open(&path).expect_err("future schema must be rejected");
        assert!(error.to_string().contains("newer than supported version 3"));
        Ok(())
    }

    #[test]
    fn live_cache_identity_reuses_exact_context_and_resets_incompatible_rows() -> Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let iex = CacheIdentity::new("alpaca:v1|feed=iex", MarketContext::us_equities());
        assert_eq!(
            storage.prepare_live_cache(&iex)?,
            CachePreparation::Initialized
        );
        assert_eq!(storage.cache_identity()?, Some(iex.clone()));
        assert_eq!(storage.prepare_live_cache(&iex)?, CachePreparation::Reused);

        let now = instant(13);
        storage.replace_universe(
            now.date_naive(),
            &[
                company(
                    "KEEP",
                    "Favorite Company",
                    Sector::Technology,
                    300.0,
                    Some(1),
                    now,
                ),
                company(
                    "DROP",
                    "Other Company",
                    Sector::Financial,
                    200.0,
                    Some(1),
                    now,
                ),
            ],
        )?;
        storage.set_favorite("KEEP", true)?;
        storage.upsert_bars(&[
            bar("KEEP", now, 101.0, 1_000.0),
            bar("DROP", now, 51.0, 500.0),
        ])?;
        storage.upsert_snapshots(&[
            snapshot("KEEP", 101.0, 100.0, 1_000.0, now),
            snapshot("DROP", 51.0, 50.0, 500.0, now),
        ])?;
        storage.upsert_news(&[NewsItem {
            id: "provider-news".to_owned(),
            headline: "Provider-specific headline".to_owned(),
            source: "Provider".to_owned(),
            published_at: now,
            url: "https://example.com/news".to_owned(),
            summary: String::new(),
            symbols: vec!["KEEP".to_owned()],
        }])?;
        storage.set_sync_checkpoint("snapshots", now)?;

        let sip = CacheIdentity::new("alpaca:v1|feed=sip", MarketContext::us_equities());
        let preparation = storage.prepare_live_cache(&sip)?;
        assert_eq!(preparation, CachePreparation::Reset);
        assert!(preparation.was_reset());
        assert_eq!(storage.cache_identity()?, Some(sip.clone()));
        assert_eq!(storage.market_context()?, Some(sip.market.clone()));
        assert_eq!(
            storage.counts()?,
            StorageCounts {
                companies: 1,
                favorites: 1,
                ..StorageCounts::default()
            }
        );
        assert!(storage.company("DROP")?.is_none());
        let favorite = storage.company("KEEP")?.expect("favorite symbol survives");
        assert_eq!(favorite.name, "KEEP");
        assert_eq!(favorite.sector, None);
        assert_eq!(favorite.exchange, "");
        assert_eq!(favorite.market_cap, None);
        assert_eq!(favorite.shares_outstanding, None);
        assert_eq!(favorite.rank, None);
        assert!(!favorite.in_universe);
        assert!(favorite.retained);
        assert_eq!(storage.favorite_symbols()?, vec!["KEEP".to_owned()]);
        assert_eq!(storage.prepare_live_cache(&sip)?, CachePreparation::Reused);

        storage.upsert_bars(&[bar("KEEP", now, 102.0, 250.0)])?;
        let other_market = MarketContext {
            id: "uk-equities".into(),
            symbol_namespace: "uk-equity".into(),
            currency: "GBP".into(),
            timezone: chrono_tz::Europe::London,
            regular_open: NaiveTime::from_hms_opt(8, 0, 0).expect("valid London-session open"),
            regular_close: NaiveTime::from_hms_opt(16, 30, 0).expect("valid London-session close"),
        };
        let london = CacheIdentity::new(sip.namespace, other_market);
        assert_eq!(
            storage.prepare_live_cache(&london)?,
            CachePreparation::Reset
        );
        assert!(storage.bars("KEEP", None, None, None, None)?.is_empty());
        assert_eq!(storage.cache_identity()?, Some(london));
        Ok(())
    }

    #[test]
    fn legacy_unstamped_rows_are_not_assumed_to_match_a_live_provider() -> Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = instant(13);
        storage.upsert_companies(&[company(
            "OLD",
            "Ambiguous Cached Company",
            Sector::Technology,
            100.0,
            Some(1),
            now,
        )])?;
        storage.upsert_bars(&[bar("OLD", now, 10.0, 100.0)])?;

        let identity = CacheIdentity::new(
            "stock-api:v1|base=https://stock.example/",
            MarketContext::default(),
        );
        assert_eq!(
            storage.prepare_live_cache(&identity)?,
            CachePreparation::Reset
        );
        assert_eq!(storage.counts()?, StorageCounts::default());
        assert_eq!(storage.cache_identity()?, Some(identity));
        Ok(())
    }

    #[test]
    fn current_market_cap_outweighs_catalog_proxy_rank() -> Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = instant(13);
        let proxy_leader = company(
            "OLD",
            "Proxy Leader",
            Sector::Technology,
            100.0,
            Some(1),
            now,
        );
        let current_leader = company(
            "NEW",
            "Current Leader",
            Sector::Technology,
            500.0,
            Some(200),
            now,
        );
        storage.replace_memberships(
            now.date_naive(),
            Sector::Technology,
            &[proxy_leader, current_leader],
        )?;

        let members = storage.memberships(Sector::Technology, None)?;
        assert_eq!(members[0].symbol, "NEW");
        assert_eq!(members[0].rank, Some(1));
        assert_eq!(members[1].symbol, "OLD");
        assert_eq!(members[1].rank, Some(2));
        Ok(())
    }

    #[test]
    fn numeric_size_proxy_competes_with_known_caps_without_becoming_market_cap() -> Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = instant(13);
        let mut candidates = (0..100)
            .map(|index| {
                company(
                    &format!("K{index:03}"),
                    &format!("Known {index}"),
                    Sector::Technology,
                    1_000.0 - f64::from(index),
                    Some(u16::try_from(index + 1).unwrap()),
                    now,
                )
            })
            .collect::<Vec<_>>();
        let mut proxy_leader = company(
            "PROXY",
            "Proxy Leader",
            Sector::Technology,
            1.0,
            Some(200),
            now,
        );
        proxy_leader.market_cap = None;
        proxy_leader.size_proxy = Some(10_000.0);
        proxy_leader.size_proxy_source = Some("sec_entity_public_float".to_owned());
        proxy_leader.size_proxy_as_of =
            Some(NaiveDate::from_ymd_opt(2026, 6, 30).expect("valid date"));
        proxy_leader.size_proxy_confidence = Some("low".to_owned());
        candidates.push(proxy_leader);

        storage.replace_memberships(now.date_naive(), Sector::Technology, &candidates)?;

        let members = storage.memberships(Sector::Technology, None)?;
        assert_eq!(members.len(), MAX_MEMBERS_PER_SECTOR);
        assert_eq!(members[0].symbol, "PROXY");
        assert_eq!(members[0].market_cap, None);
        assert_eq!(members[0].size_proxy, Some(10_000.0));
        assert_eq!(
            members[0].size_proxy_source.as_deref(),
            Some("sec_entity_public_float")
        );
        assert!(!members.iter().any(|company| company.symbol == "K099"));
        Ok(())
    }

    #[test]
    fn batches_are_atomic_and_newer_snapshots_win() -> Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = instant(13);
        storage.upsert_companies(&[company(
            "AAPL",
            "Apple",
            Sector::Technology,
            3_000.0,
            Some(1),
            now,
        )])?;

        let error = storage.upsert_bars(&[
            bar("AAPL", now, 100.0, 10.0),
            bar("MISSING", now, 50.0, 10.0),
        ]);
        assert!(error.is_err());
        assert!(storage.bars("AAPL", None, None, None, None)?.is_empty());

        storage.upsert_bars(&[bar("AAPL", now, 100.0, 10.0)])?;
        storage.upsert_bars(&[bar("AAPL", now, 105.0, 20.0)])?;
        assert_eq!(
            storage.bars("AAPL", Some("1Day"), None, None, None)?[0].close,
            105.0
        );

        storage.upsert_snapshots(&[snapshot("AAPL", 105.0, 100.0, 20.0, now)])?;
        storage.upsert_snapshots(&[snapshot("AAPL", 1.0, 100.0, 1.0, instant(12))])?;
        assert_eq!(storage.snapshot("AAPL")?.unwrap().price, Some(105.0));
        Ok(())
    }

    #[test]
    fn cancelled_bar_batch_rolls_back_partial_inserts() -> Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = instant(13);
        storage.upsert_companies(&[company(
            "AAPL",
            "Apple",
            Sector::Technology,
            3_000.0,
            Some(1),
            now,
        )])?;
        let checks = Cell::new(0_usize);

        let inserted = storage.upsert_bars_until_cancelled(
            &[
                bar("AAPL", now, 100.0, 10.0),
                bar("AAPL", instant(14), 101.0, 11.0),
            ],
            || {
                checks.set(checks.get() + 1);
                checks.get() > 1
            },
        )?;

        assert_eq!(inserted, None);
        assert!(storage.bars("AAPL", None, None, None, None)?.is_empty());
        Ok(())
    }

    #[test]
    fn heatmap_detail_news_and_favorites_share_cached_data() -> Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = instant(13);
        let companies = [
            company("AAA", "Alpha", Sector::Technology, 300.0, Some(1), now),
            company("BBB", "Beta", Sector::Technology, 200.0, Some(2), now),
            company("CCC", "Gamma", Sector::Technology, 100.0, Some(3), now),
        ];
        storage.replace_universe(now.date_naive(), &companies)?;
        storage.upsert_bars(&[
            bar("AAA", instant(5), 100.0, 10.0),
            bar("AAA", now, 110.0, 100.0),
            bar("BBB", instant(5), 100.0, 10.0),
            bar("BBB", now, 90.0, 500.0),
            bar("CCC", instant(5), 100.0, 10.0),
            bar("CCC", now, 105.0, 200.0),
        ])?;
        storage.upsert_snapshots(&[
            snapshot("AAA", 110.0, 100.0, 100.0, now),
            snapshot("BBB", 90.0, 100.0, 500.0, now),
            snapshot("CCC", 105.0, 100.0, 200.0, now),
        ])?;
        storage.upsert_news(&[NewsItem {
            id: "article-1".to_owned(),
            headline: "Alpha ships a product".to_owned(),
            source: "Newswire".to_owned(),
            published_at: now,
            url: "https://example.test/article-1".to_owned(),
            summary: "A concise summary".to_owned(),
            symbols: vec!["AAA".to_owned(), "CCC".to_owned()],
        }])?;
        storage.set_favorite("ccc", true)?;

        let gainers = storage.heatmap_tiles(
            DateRange::Week,
            SortMode::Gainers,
            Some(Sector::Technology),
            false,
            now,
        )?;
        assert_eq!(
            gainers
                .iter()
                .map(|tile| tile.company.symbol.as_str())
                .collect::<Vec<_>>(),
            ["AAA", "CCC", "BBB"]
        );
        assert!(gainers[1].starred);
        assert!(
            gainers[0]
                .period_return
                .is_some_and(|value| (value - 0.1).abs() < f64::EPSILON * 4.0)
        );

        let by_volume = storage.heatmap_tiles(
            DateRange::Day,
            SortMode::Volume,
            Some(Sector::Technology),
            false,
            now,
        )?;
        assert_eq!(
            by_volume
                .iter()
                .map(|tile| tile.company.symbol.as_str())
                .collect::<Vec<_>>(),
            ["BBB", "CCC", "AAA"]
        );
        let by_volume_ascending = storage.heatmap_tiles_ordered(
            DateRange::Day,
            SortMode::Volume,
            false,
            Some(Sector::Technology),
            false,
            now,
        )?;
        assert_eq!(
            by_volume_ascending
                .iter()
                .map(|tile| tile.company.symbol.as_str())
                .collect::<Vec<_>>(),
            ["AAA", "CCC", "BBB"]
        );
        let alphabetical_descending = storage.heatmap_tiles_ordered(
            DateRange::Day,
            SortMode::Alphabetical,
            true,
            Some(Sector::Technology),
            false,
            now,
        )?;
        assert_eq!(
            alphabetical_descending
                .iter()
                .map(|tile| tile.company.symbol.as_str())
                .collect::<Vec<_>>(),
            ["CCC", "BBB", "AAA"]
        );

        let detail = storage
            .ticker_detail("aaa", DateRange::Week, now, 10)?
            .expect("known company");
        assert_eq!(detail.company.symbol, "AAA");
        assert_eq!(detail.bars.len(), 2);
        assert_eq!(detail.news[0].headline, "Alpha ships a product");
        assert_eq!(detail.sector_rank, Some(1));
        assert!(
            detail
                .period_return
                .is_some_and(|value| (value - 0.1).abs() < f64::EPSILON * 4.0)
        );
        assert!(!detail.starred);
        Ok(())
    }

    #[test]
    fn tile_period_values_share_the_displayed_snapshot_endpoint() -> Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = instant(13);
        storage.replace_universe(
            now.date_naive(),
            &[company(
                "AAA",
                "Alpha",
                Sector::Technology,
                300.0,
                Some(1),
                now,
            )],
        )?;
        storage.upsert_bars(&[
            bar("AAA", instant(5), 100.0, 10.0),
            bar("AAA", now, 110.0, 100.0),
        ])?;
        storage.upsert_snapshots(&[snapshot("AAA", 125.0, 120.0, 100.0, now)])?;

        let week = storage.heatmap_tiles(
            DateRange::Week,
            SortMode::Gainers,
            Some(Sector::Technology),
            false,
            now,
        )?;
        assert_eq!(week[0].price, Some(125.0));
        assert_eq!(week[0].period_start_price, Some(100.0));
        assert!(
            week[0]
                .period_return
                .is_some_and(|value| (value - 0.25).abs() < f64::EPSILON * 4.0)
        );
        assert_eq!(week[0].absolute_change(), Some(25.0));

        let day = storage.heatmap_tiles(
            DateRange::Day,
            SortMode::Gainers,
            Some(Sector::Technology),
            false,
            now,
        )?;
        assert_eq!(day[0].price, Some(125.0));
        assert_eq!(day[0].period_start_price, Some(120.0));
        assert!(
            day[0]
                .period_return
                .is_some_and(|value| (value - (125.0 / 120.0 - 1.0)).abs() < f64::EPSILON * 4.0)
        );
        assert_eq!(day[0].absolute_change(), Some(5.0));

        let detail = storage
            .ticker_detail("AAA", DateRange::Week, now, 0)?
            .expect("known company");
        assert_eq!(detail.period_start_price, Some(100.0));
        assert_eq!(detail.period_end_price, Some(125.0));
        assert_eq!(detail.period_end_at, Some(now));
        assert!(
            detail
                .period_return
                .is_some_and(|value| (value - 0.25).abs() < f64::EPSILON * 4.0)
        );
        Ok(())
    }

    #[test]
    fn heatmap_volume_tracks_the_selected_range_and_changes_order() -> Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = instant(13);
        storage.replace_universe(
            now.date_naive(),
            &[
                company("AAA", "Alpha", Sector::Technology, 300.0, Some(1), now),
                company("BBB", "Beta", Sector::Technology, 200.0, Some(2), now),
            ],
        )?;
        let mut weekly_aaa = bar("AAA", now, 103.0, 5.0);
        weekly_aaa.timeframe = "1Week".to_owned();
        let mut weekly_bbb = bar("BBB", now, 103.0, 900.0);
        weekly_bbb.timeframe = "1Week".to_owned();
        storage.upsert_bars(&[
            bar("AAA", instant(2), 100.0, 800.0),
            bar("AAA", instant(8), 101.0, 10.0),
            bar("AAA", instant(12), 102.0, 10.0),
            bar("AAA", now, 103.0, 10.0),
            bar("BBB", instant(2), 100.0, 10.0),
            bar("BBB", instant(8), 101.0, 100.0),
            bar("BBB", instant(12), 102.0, 100.0),
            bar("BBB", now, 103.0, 100.0),
            weekly_aaa,
            weekly_bbb,
        ])?;
        storage.upsert_snapshots(&[
            snapshot("AAA", 103.0, 102.0, 500.0, now),
            snapshot("BBB", 103.0, 102.0, 50.0, now),
        ])?;

        let day = storage.heatmap_tiles(
            DateRange::Day,
            SortMode::Volume,
            Some(Sector::Technology),
            false,
            now,
        )?;
        assert_eq!(day[0].company.symbol, "AAA");
        assert_eq!(day[0].volume, Some(500.0));
        assert_eq!(day[1].volume, Some(50.0));

        let week = storage.heatmap_tiles(
            DateRange::Week,
            SortMode::Volume,
            Some(Sector::Technology),
            false,
            now,
        )?;
        assert_eq!(week[0].company.symbol, "BBB");
        assert_eq!(week[0].volume, Some(300.0));
        assert_eq!(week[1].volume, Some(30.0));

        let month = storage.heatmap_tiles(
            DateRange::Month,
            SortMode::Volume,
            Some(Sector::Technology),
            false,
            now,
        )?;
        assert_eq!(month[0].company.symbol, "AAA");
        assert_eq!(month[0].volume, Some(830.0));
        assert_eq!(month[1].volume, Some(310.0));

        let five_years = storage.heatmap_tiles(
            DateRange::FiveYears,
            SortMode::Volume,
            Some(Sector::Technology),
            false,
            now,
        )?;
        assert_eq!(five_years[0].company.symbol, "BBB");
        assert_eq!(five_years[0].volume, Some(900.0));
        assert_eq!(five_years[1].volume, Some(5.0));
        Ok(())
    }

    #[test]
    fn day_volume_falls_back_to_inclusive_daily_bars_but_not_weekly_bars() -> Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = instant(13);
        storage.replace_universe(
            now.date_naive(),
            &[
                company("AAA", "Alpha", Sector::Technology, 300.0, Some(1), now),
                company("BBB", "Beta", Sector::Technology, 200.0, Some(2), now),
            ],
        )?;
        let mut weekly_bbb = bar("BBB", now, 103.0, 900.0);
        weekly_bbb.timeframe = "1Week".to_owned();
        storage.upsert_bars(&[
            bar("AAA", DateRange::Day.cutoff(now), 102.0, 20.0),
            bar("AAA", now, 103.0, 30.0),
            weekly_bbb,
        ])?;
        let mut snapshot_without_volume = snapshot("AAA", 103.0, 102.0, 500.0, now);
        snapshot_without_volume.volume = None;
        storage.upsert_snapshots(&[snapshot_without_volume])?;

        let tiles = storage.heatmap_tiles(
            DateRange::Day,
            SortMode::Alphabetical,
            Some(Sector::Technology),
            false,
            now,
        )?;

        assert_eq!(tiles[0].company.symbol, "AAA");
        assert_eq!(tiles[0].volume, Some(50.0));
        assert_eq!(tiles[1].company.symbol, "BBB");
        assert_eq!(tiles[1].volume, None);
        Ok(())
    }

    #[test]
    fn period_endpoint_uses_the_newer_price_source_and_its_timestamp() -> Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = instant(13);
        storage.replace_universe(
            now.date_naive(),
            &[company(
                "AAA",
                "Alpha",
                Sector::Technology,
                300.0,
                Some(1),
                now,
            )],
        )?;
        storage.upsert_bars(&[
            bar("AAA", instant(5), 100.0, 10.0),
            bar("AAA", now, 110.0, 100.0),
        ])?;
        storage.upsert_snapshots(&[snapshot("AAA", 125.0, 120.0, 999.0, instant(12))])?;

        let tile = storage
            .heatmap_tiles(
                DateRange::Week,
                SortMode::Gainers,
                Some(Sector::Technology),
                false,
                now,
            )?
            .remove(0);
        assert_eq!(tile.price, Some(110.0));
        assert_eq!(tile.period_start_price, Some(100.0));
        assert!(
            tile.period_return
                .is_some_and(|value| (value - 0.1).abs() < f64::EPSILON * 4.0)
        );
        assert_eq!(tile.volume, Some(100.0));
        assert_eq!(tile.updated_at, Some(now));
        assert!(!tile.stale);

        let detail = storage
            .ticker_detail("AAA", DateRange::Week, now, 0)?
            .expect("known company");
        assert_eq!(detail.period_end_price, Some(110.0));
        assert_eq!(detail.period_end_at, Some(now));
        assert!(
            detail
                .period_return
                .is_some_and(|value| (value - 0.1).abs() < f64::EPSILON * 4.0)
        );
        let all_detail = storage
            .ticker_detail("AAA", DateRange::All, now, 0)?
            .expect("known company");
        assert_eq!(all_detail.period_start_price, Some(100.0));
        assert_eq!(all_detail.period_start_at, Some(instant(5)));
        Ok(())
    }

    #[test]
    fn no_trade_placeholders_do_not_extend_price_history() -> Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = instant(20);
        storage.replace_universe(
            now.date_naive(),
            &[company(
                "AAA",
                "Alpha",
                Sector::Technology,
                300.0,
                Some(1),
                now,
            )],
        )?;

        let mut weekly = bar("AAA", instant(2), 90.0, 1_000.0);
        weekly.timeframe = "1Week".to_owned();
        let daily = bar("AAA", instant(10), 100.0, 500.0);
        let hourly_placeholder = no_trade_bar("AAA", "1Hour", instant(18), 150.0, None);
        let daily_placeholder = no_trade_bar("AAA", "1Day", instant(19), 150.0, Some(0));
        storage.upsert_bars(&[weekly, daily, hourly_placeholder, daily_placeholder])?;
        storage.upsert_snapshots(&[snapshot("AAA", 101.0, 100.0, 600.0, instant(11))])?;

        let raw_bars = storage.bars("AAA", None, None, None, None)?;
        assert_eq!(raw_bars.len(), 4);
        assert_eq!(
            raw_bars
                .iter()
                .filter(|bar| !bar.is_price_observation())
                .count(),
            2
        );
        assert_eq!(
            storage.latest_bar_timestamp("AAA", "1Day")?,
            Some(instant(10))
        );
        assert_eq!(storage.latest_bar_timestamp("AAA", "1Hour")?, None);

        let connection = storage.connection()?;
        let mut statement = connection.prepare_cached(TIMEFRAME_EXISTS_SQL)?;
        assert_eq!(
            choose_timeframe(&mut statement, DateRange::Month, "AAA")?,
            "1Day"
        );
        drop(statement);
        drop(connection);

        let tile = storage
            .heatmap_tiles(
                DateRange::Month,
                SortMode::Gainers,
                Some(Sector::Technology),
                false,
                now,
            )?
            .remove(0);
        assert_eq!(tile.price, Some(101.0));
        assert_eq!(tile.period_start_price, Some(100.0));
        assert_eq!(tile.updated_at, Some(instant(11)));
        assert!(tile.stale);

        let detail = storage
            .ticker_detail("AAA", DateRange::Month, now, 0)?
            .expect("known company");
        assert_eq!(detail.bars.len(), 1);
        assert_eq!(detail.bars[0].timestamp, instant(10));
        assert_eq!(detail.period_start_price, Some(100.0));
        assert_eq!(detail.period_end_price, Some(101.0));
        assert_eq!(detail.period_end_at, Some(instant(11)));
        assert_eq!(detail.history_start_at, Some(instant(2)));
        assert_eq!(detail.history_end_at, Some(instant(10)));
        Ok(())
    }

    #[test]
    fn price_less_snapshot_does_not_refresh_an_old_bar_price() -> Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = instant(13);
        storage.replace_universe(
            now.date_naive(),
            &[company(
                "AAA",
                "Alpha",
                Sector::Technology,
                300.0,
                Some(1),
                now,
            )],
        )?;
        storage.upsert_bars(&[bar("AAA", instant(5), 100.0, 10.0)])?;
        let mut current_snapshot = snapshot("AAA", 125.0, 120.0, 100.0, now);
        current_snapshot.price = None;
        storage.upsert_snapshots(&[current_snapshot])?;

        let tile = storage
            .heatmap_tiles(
                DateRange::Week,
                SortMode::Gainers,
                Some(Sector::Technology),
                false,
                now,
            )?
            .remove(0);
        assert_eq!(tile.price, Some(100.0));
        assert_eq!(tile.volume, None);
        assert_eq!(tile.updated_at, Some(instant(5)));
        assert!(tile.stale);
        Ok(())
    }

    #[test]
    fn benchmark_tiles_use_retained_sectorless_company_data() -> Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = instant(13);
        let benchmark = MarketBenchmark::ALL[0].company(now);
        storage.upsert_companies(&[benchmark])?;
        storage.upsert_bars(&[
            bar("SPY", instant(5), 600.0, 1_000.0),
            bar("SPY", now, 612.0, 2_000.0),
        ])?;
        storage.upsert_snapshots(&[snapshot("SPY", 612.0, 606.0, 2_000.0, now)])?;

        let tiles = storage.benchmark_tiles(DateRange::Week, now)?;

        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].company.symbol, "SPY");
        assert_eq!(tiles[0].company.sector, None);
        assert_eq!(tiles[0].price, Some(612.0));
        assert_eq!(tiles[0].period_start_price, Some(600.0));
        assert_eq!(tiles[0].absolute_change(), Some(12.0));
        assert!(
            tiles[0]
                .period_return
                .is_some_and(|value| (value - 0.02).abs() < f64::EPSILON * 4.0)
        );
        assert!(!tiles[0].stale);
        Ok(())
    }

    #[test]
    fn live_transition_removes_demo_rows_and_preserves_alpaca_cache_and_favorites() -> Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = instant(13);
        let mut simulated = company(
            "AAA",
            "Alpha",
            Sector::Technology,
            3_000_000.0,
            Some(1),
            now,
        );
        simulated.raw_sector = Some("Technology · SIMULATED DEMO".to_owned());
        simulated.in_universe = true;
        simulated.retained = false;
        storage.replace_universe(now.date_naive(), &[simulated])?;
        storage.set_favorite("AAA", true)?;

        let mut demo_bar = bar("AAA", now - chrono::Duration::hours(4), 314.0, 1_000.0);
        demo_bar.source = "demo".to_owned();
        let mut live_bar = bar("AAA", now - chrono::Duration::hours(3), 518.0, 2_000.0);
        live_bar.source = "alpaca".to_owned();
        storage.upsert_bars(&[demo_bar, live_bar])?;
        storage.upsert_snapshots(&[snapshot("AAA", 518.0, 516.0, 2_000.0, now)])?;
        storage.upsert_news(&[
            NewsItem {
                id: "demo-AAA-outlook".to_owned(),
                headline: "[SIMULATED] Alpha outlook".to_owned(),
                source: "SIMULATED · DemoWire".to_owned(),
                published_at: now,
                url: "https://example.invalid/demo".to_owned(),
                summary: "Simulated".to_owned(),
                symbols: vec!["AAA".to_owned()],
            },
            NewsItem {
                id: "alpaca-article".to_owned(),
                headline: "Alpha live headline".to_owned(),
                source: "Benzinga".to_owned(),
                published_at: now - chrono::Duration::minutes(1),
                url: "https://example.test/live".to_owned(),
                summary: "Live".to_owned(),
                symbols: vec!["AAA".to_owned()],
            },
        ])?;
        storage.set_sync_checkpoint(crate::demo::CHECKPOINT_SCOPE, now)?;
        storage.set_sync_checkpoint("snapshots", now)?;
        storage.set_sync_checkpoint("history:1Day:2Y:symbol:AAA", now)?;

        assert!(storage.purge_demo_data_for_live()?);

        let bars = storage.bars("AAA", Some("1Day"), None, None, None)?;
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].source, "alpaca");
        assert_eq!(bars[0].close, 518.0);
        let news = storage.news(Some("AAA"), 10)?;
        assert_eq!(news.len(), 1);
        assert_eq!(news[0].id, "alpaca-article");
        assert!(storage.snapshot("AAA")?.is_none());
        assert!(storage.memberships(Sector::Technology, None)?.is_empty());
        assert!(storage.is_favorite("AAA")?);

        let company = storage.company("AAA")?.expect("company remains cached");
        assert_eq!(company.market_cap, None);
        assert_eq!(company.shares_outstanding, None);
        assert!(!company.in_universe);
        assert!(company.retained);
        assert_eq!(
            storage.sync_checkpoint("history:1Day:2Y:symbol:AAA")?,
            Some(now)
        );
        assert_eq!(storage.sync_checkpoint("snapshots")?, None);
        assert_eq!(
            storage.sync_checkpoint(crate::demo::CHECKPOINT_SCOPE)?,
            None
        );
        assert!(!storage.purge_demo_data_for_live()?);
        Ok(())
    }

    #[test]
    fn search_checkpoints_concurrent_connections_and_reset() -> Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = instant(13);
        storage.upsert_companies(&[
            company(
                "CAT",
                "Caterpillar",
                Sector::Industrial,
                100.0,
                Some(1),
                now,
            ),
            company("C", "Citigroup", Sector::Financial, 90.0, Some(1), now),
            company(
                "DOG",
                "Catalog Systems",
                Sector::Technology,
                5.0,
                Some(90),
                now,
            ),
        ])?;
        assert_eq!(storage.search("cat", 10)?[0].symbol, "CAT");
        storage.set_sync_checkpoint("snapshots", now)?;
        storage.set_sync_checkpoints(
            &[
                "history:1Week:all:symbol:CAT".to_owned(),
                "history:1Week:all:symbol:C".to_owned(),
            ],
            now,
        )?;
        assert_eq!(storage.sync_checkpoint("snapshots")?, Some(now));
        assert_eq!(storage.sync_checkpoint("history")?, None);
        assert_eq!(
            storage.sync_checkpoint_scopes("history:1Week:all")?,
            HashSet::from([
                "history:1Week:all:symbol:C".to_owned(),
                "history:1Week:all:symbol:CAT".to_owned(),
            ])
        );

        let handles = (0_u16..4)
            .map(|index| {
                let storage = storage.clone();
                thread::spawn(move || {
                    storage.upsert_companies(&[company(
                        &format!("T{index}"),
                        &format!("Thread {index}"),
                        Sector::Services,
                        f64::from(index),
                        Some(index + 1),
                        now,
                    )])
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().expect("writer thread should not panic")?;
        }
        assert_eq!(storage.counts()?.companies, 7);

        storage.toggle_favorite("CAT")?;
        assert_eq!(storage.favorite_symbols()?, vec!["CAT".to_owned()]);
        assert!(!storage.toggle_favorite("CAT")?);
        storage.reset_demo_data()?;
        assert_eq!(storage.counts()?, StorageCounts::default());
        Ok(())
    }

    #[test]
    fn timeframe_selection_uses_indexed_symbol_probes_and_preserves_fallbacks() -> Result<()> {
        let directory = tempdir()?;
        let storage = Storage::open(directory.path().join("market.sqlite3"))?;
        let now = instant(13);
        storage.upsert_companies(&[company(
            "AAA",
            "Alpha",
            Sector::Technology,
            100.0,
            Some(1),
            now,
        )])?;
        let daily = bar("AAA", now - chrono::Duration::days(1), 100.0, 1_000.0);
        let mut weekly = bar("AAA", now - chrono::Duration::days(7), 90.0, 5_000.0);
        weekly.timeframe = "1Week".to_owned();
        storage.upsert_bars(&[daily, weekly])?;

        let connection = storage.connection()?;
        let mut statement = connection.prepare_cached(TIMEFRAME_EXISTS_SQL)?;
        assert_eq!(
            choose_timeframe(&mut statement, DateRange::Day, "AAA")?,
            "1Day"
        );
        assert_eq!(
            choose_timeframe(&mut statement, DateRange::FiveYears, "AAA")?,
            "1Week"
        );
        assert_eq!(
            choose_timeframe(&mut statement, DateRange::Month, "MISSING")?,
            "1Hour"
        );
        drop(statement);

        let mut plan = connection.prepare(
            "EXPLAIN QUERY PLAN
             SELECT EXISTS(
                 SELECT 1 FROM bars
                 WHERE symbol = ?1 AND timeframe = ?2
                   AND NOT (
                       volume = 0 AND COALESCE(trade_count, 0) = 0
                       AND open = high AND high = low AND low = close
                   )
                 LIMIT 1
             )",
        )?;
        let details = plan
            .query_map(params!["AAA", "1Day"], |row| row.get::<_, String>(3))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        assert!(
            details
                .iter()
                .any(|detail| { detail.contains("SEARCH bars") && detail.contains("USING INDEX") }),
            "timeframe existence probe must use a bars index: {details:?}"
        );
        Ok(())
    }
}
