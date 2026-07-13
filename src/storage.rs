use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{
    Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params, types::Type,
};

use crate::domain::{
    Bar, Company, DateRange, MarketTile, NewsItem, Sector, Snapshot, SortMode, TickerDetail,
};

const SCHEMA_VERSION: i64 = 1;
const STALE_AFTER_HOURS: i64 = 72;
const MAX_MEMBERS_PER_SECTOR: usize = 100;

const COMPANY_COLUMNS: &str = "
    symbol, name, sector, raw_sector, exchange, industry, market_cap,
    shares_outstanding, rank, description, in_universe, retained, updated_at
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

#[derive(Debug, Clone)]
pub struct Storage {
    path: PathBuf,
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
    close: Option<f64>,
    volume: Option<f64>,
    updated_at: Option<DateTime<Utc>>,
    period_return: Option<f64>,
}

type PeriodMetricRow = (
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<i64>,
);

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
                    COALESCE(m.market_cap, c.market_cap), c.shares_outstanding,
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
        if bars.is_empty() {
            return Ok(0);
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
        Ok(bars.len())
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
            "SELECT MAX(timestamp) FROM bars WHERE symbol = ?1 AND timeframe = ?2",
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
                market_cap IS NULL,
                market_cap DESC,
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
        let scope = scope.trim();
        if scope.is_empty() {
            bail!("sync checkpoint scope must not be empty");
        }
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO sync_checkpoints (scope, completed_at, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(scope) DO UPDATE SET
                completed_at = excluded.completed_at,
                updated_at = excluded.updated_at",
            params![
                scope,
                timestamp_millis(completed_at),
                timestamp_millis(Utc::now())
            ],
        )?;
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

    pub fn heatmap_tiles(
        &self,
        range: DateRange,
        sort: SortMode,
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
        let available_timeframes = load_available_timeframes(&connection)?;
        let mut metric_statement = connection.prepare_cached(
            "SELECT
                (SELECT close FROM bars
                 WHERE symbol = ?1 AND timeframe = ?2 AND timestamp <= ?3
                 ORDER BY timestamp DESC LIMIT 1),
                (SELECT close FROM bars
                 WHERE symbol = ?1 AND timeframe = ?2
                   AND timestamp >= ?3 AND timestamp <= ?4
                 ORDER BY timestamp ASC LIMIT 1),
                (SELECT close FROM bars
                 WHERE symbol = ?1 AND timeframe = ?2 AND timestamp <= ?4
                 ORDER BY timestamp DESC LIMIT 1),
                (SELECT volume FROM bars
                 WHERE symbol = ?1 AND timeframe = ?2 AND timestamp <= ?4
                 ORDER BY timestamp DESC LIMIT 1),
                (SELECT timestamp FROM bars
                 WHERE symbol = ?1 AND timeframe = ?2 AND timestamp <= ?4
                 ORDER BY timestamp DESC LIMIT 1)",
        )?;
        let cutoff = range.cutoff(now);
        let mut tiles = Vec::with_capacity(companies.len());
        for company in companies {
            let timeframe = choose_timeframe(range, available_timeframes.get(&company.symbol));
            let metric = load_period_metric(
                &mut metric_statement,
                &company.symbol,
                timeframe,
                cutoff,
                now,
            )?;
            let snapshot = snapshots.get(&company.symbol);
            let period_return = if range == DateRange::Day {
                snapshot
                    .and_then(Snapshot::day_return)
                    .or(metric.period_return)
            } else {
                metric.period_return
            };
            let price = snapshot.and_then(|value| value.price).or(metric.close);
            let volume = snapshot.and_then(|value| value.volume).or(metric.volume);
            let updated_at = snapshot.map(|value| value.updated_at).or(metric.updated_at);
            let stale = updated_at.is_none_or(|updated| {
                now.signed_duration_since(updated).num_hours() > STALE_AFTER_HOURS
            });
            let starred = favorite_symbols.contains(&company.symbol);
            tiles.push(MarketTile {
                company,
                price,
                period_return,
                volume,
                starred,
                stale,
                updated_at,
            });
        }
        sort_and_limit_tiles(&mut tiles, sort, sector, favorites_only);
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
        let connection = self.connection()?;
        let available = load_available_timeframes(&connection)?;
        let timeframe = choose_timeframe(range, available.get(&company.symbol));
        let mut metric_statement = connection.prepare_cached(
            "SELECT
                (SELECT close FROM bars
                 WHERE symbol = ?1 AND timeframe = ?2 AND timestamp <= ?3
                 ORDER BY timestamp DESC LIMIT 1),
                (SELECT close FROM bars
                 WHERE symbol = ?1 AND timeframe = ?2
                   AND timestamp >= ?3 AND timestamp <= ?4
                 ORDER BY timestamp ASC LIMIT 1),
                (SELECT close FROM bars
                 WHERE symbol = ?1 AND timeframe = ?2 AND timestamp <= ?4
                 ORDER BY timestamp DESC LIMIT 1),
                (SELECT volume FROM bars
                 WHERE symbol = ?1 AND timeframe = ?2 AND timestamp <= ?4
                 ORDER BY timestamp DESC LIMIT 1),
                (SELECT timestamp FROM bars
                 WHERE symbol = ?1 AND timeframe = ?2 AND timestamp <= ?4
                 ORDER BY timestamp DESC LIMIT 1)",
        )?;
        let metric = load_period_metric(
            &mut metric_statement,
            &company.symbol,
            timeframe,
            range.cutoff(now),
            now,
        )?;
        drop(metric_statement);
        drop(connection);
        let bars = self.bars(
            &company.symbol,
            Some(timeframe),
            Some(range.cutoff(now)),
            Some(now),
            None,
        )?;
        let snapshot = self.snapshot(&company.symbol)?;
        let period_return = if range == DateRange::Day {
            snapshot
                .as_ref()
                .and_then(Snapshot::day_return)
                .or(metric.period_return)
        } else {
            metric.period_return
        };
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
            period_return,
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
        transaction.commit().context("could not commit cache reset")
    }
}

fn upsert_company(
    transaction: &Transaction<'_>,
    company: &Company,
    force_in_universe: Option<bool>,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO companies (
            symbol, name, sector, raw_sector, exchange, industry, market_cap,
            shares_outstanding, rank, description, in_universe, retained, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(symbol) DO UPDATE SET
            name = excluded.name,
            sector = excluded.sector,
            raw_sector = excluded.raw_sector,
            exchange = excluded.exchange,
            industry = excluded.industry,
            market_cap = excluded.market_cap,
            shares_outstanding = excluded.shares_outstanding,
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
            company.shares_outstanding,
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
        "INSERT INTO sector_memberships (as_of_date, sector, symbol, rank, market_cap)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for (position, company) in companies.iter().enumerate() {
        statement.execute(params![
            as_of.to_string(),
            sector_key(sector),
            normalize_symbol(&company.symbol)?,
            i64::try_from(position + 1).unwrap_or(i64::MAX),
            company.market_cap,
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
        compare_optional_f64_desc(left.market_cap, right.market_cap)
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
                COALESCE(memberships.market_cap, c.market_cap), c.shares_outstanding,
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

fn load_available_timeframes(connection: &Connection) -> Result<HashMap<String, HashSet<String>>> {
    let mut statement = connection.prepare("SELECT DISTINCT symbol, timeframe FROM bars")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut available: HashMap<String, HashSet<String>> = HashMap::new();
    for row in rows {
        let (symbol, timeframe) = row?;
        available.entry(symbol).or_default().insert(timeframe);
    }
    Ok(available)
}

fn choose_timeframe(range: DateRange, available: Option<&HashSet<String>>) -> &'static str {
    let candidates: &[&str] = match range {
        DateRange::Day => &["5Min", "15Min", "1Hour", "1Day"],
        DateRange::Week => &["1Hour", "1Day", "15Min", "5Min", "1Week"],
        DateRange::Month => &["1Hour", "1Day", "1Week"],
        DateRange::ThreeMonths | DateRange::SixMonths => &["1Day", "1Hour", "1Week"],
        DateRange::Year => &["1Day", "1Week", "1Hour"],
        DateRange::FiveYears => &["1Week", "1Day"],
    };
    available
        .and_then(|available| {
            candidates
                .iter()
                .copied()
                .find(|candidate| available.contains(*candidate))
        })
        .unwrap_or_else(|| range.preferred_timeframe())
}

fn load_period_metric(
    statement: &mut rusqlite::CachedStatement<'_>,
    symbol: &str,
    timeframe: &str,
    cutoff: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<PeriodMetric> {
    let (before, after, close, volume, timestamp): PeriodMetricRow = statement.query_row(
        params![
            symbol,
            timeframe,
            timestamp_millis(cutoff),
            timestamp_millis(now)
        ],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    let baseline = before.or(after);
    let period_return = baseline
        .filter(|value| *value != 0.0)
        .zip(close)
        .map(|(baseline, latest)| latest / baseline - 1.0);
    Ok(PeriodMetric {
        close,
        volume,
        updated_at: timestamp.map(datetime_from_millis).transpose()?,
        period_return,
    })
}

fn sort_and_limit_tiles(
    tiles: &mut Vec<MarketTile>,
    sort: SortMode,
    selected_sector: Option<Sector>,
    favorites_only: bool,
) {
    let compare = |left: &MarketTile, right: &MarketTile| {
        compare_tiles(left, right, sort)
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

fn compare_tiles(left: &MarketTile, right: &MarketTile, sort: SortMode) -> Ordering {
    match sort {
        SortMode::MarketCap => {
            compare_optional_f64_desc(left.company.market_cap, right.company.market_cap)
                .then_with(|| left.company.rank.cmp(&right.company.rank))
        }
        SortMode::Gainers => compare_optional_f64_desc(left.period_return, right.period_return),
        SortMode::Volume => compare_optional_f64_desc(left.volume, right.volume),
        SortMode::Alphabetical => left.company.symbol.cmp(&right.company.symbol),
    }
}

fn compare_optional_f64_desc(left: Option<f64>, right: Option<f64>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => right.total_cmp(&left),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn table_count(connection: &Connection, table: &str) -> Result<usize> {
    let count: i64 = connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })?;
    usize::try_from(count).context("SQLite returned a negative row count")
}

fn company_from_row(row: &Row<'_>) -> rusqlite::Result<Company> {
    let rank = row
        .get::<_, Option<i64>>(8)?
        .map(|value| {
            u16::try_from(value).map_err(|error| conversion_error(8, Type::Integer, error))
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
        shares_outstanding: row.get(7)?,
        rank,
        description: row.get(9)?,
        in_universe: row.get(10)?,
        retained: row.get(11)?,
        updated_at: datetime_from_millis(row.get(12)?)?,
    })
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
    use std::thread;

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
            shares_outstanding: Some(1_000_000.0),
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
        assert_eq!(storage.schema_version()?, 1);
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

        assert_eq!(
            storage.latest_membership_date(Some(Sector::Technology))?,
            Some(july)
        );
        assert_eq!(
            storage
                .memberships(Sector::Technology, Some(june))?
                .into_iter()
                .map(|value| value.symbol)
                .collect::<Vec<_>>(),
            vec!["AAPL".to_owned(), "MSFT".to_owned()]
        );
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

        let detail = storage
            .ticker_detail("aaa", DateRange::Week, now, 10)?
            .expect("known company");
        assert_eq!(detail.company.symbol, "AAA");
        assert_eq!(detail.bars.len(), 1);
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
        assert_eq!(storage.sync_checkpoint("snapshots")?, Some(now));
        assert_eq!(storage.sync_checkpoint("history")?, None);

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
}
