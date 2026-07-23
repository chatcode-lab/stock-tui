# Cache And Synchronization

`stock-tui` treats SQLite as the source of truth for every rendered market
view. This makes startup immediate after the first population, keeps favorites
available offline, and bounds repeated historical requests.

## Cache Location

The default database is `market.sqlite3` in the platform application-data
directory selected by the Rust `directories` crate for organization
`chatcode-lab` and application `stock-tui`.

Typical locations are:

| Platform | Default application data path |
| --- | --- |
| Linux | `$XDG_DATA_HOME/stock-tui/market.sqlite3`, normally `~/.local/share/stock-tui/market.sqlite3` |
| macOS | `~/Library/Application Support/com.chatcode-lab.stock-tui/market.sqlite3` |
| Windows | The per-user roaming application-data directory for `chatcode-lab/stock-tui`, ending in `data/market.sqlite3` |

Platform conventions can vary. Use `stock-tui --print-config` for the exact
resolved `db_path`, or override it with `--db` / `STOCK_TUI_DB_PATH`.

The application also creates platform config and cache directories. The
configuration file is `<config_dir>/config.toml`; the SQLite database belongs
in the data directory, not the disposable cache directory. Daily diagnostic
logs are written below `<cache_dir>/logs`.

## SQLite Settings

- Schema version is stored in `PRAGMA user_version`.
- Current schema version: 1.
- Journal mode: WAL.
- Foreign keys: enabled on every connection.
- Busy timeout: 30 seconds.
- Batch writes use immediate transactions.
- A binary refuses to open a schema newer than it understands.

WAL creates adjacent `-wal` and `-shm` files while the database is open. For a
consistent backup, stop `stock-tui` first and copy the database together with
any SQLite sidecars, or use SQLite's online backup tooling.

## Schema

### `companies`

One normalized row per symbol. It stores name, normalized and raw sector,
exchange, industry, optional market cap and shares outstanding, catalog rank,
description, current-universe and retained flags, and metadata update time.

The symbol is the primary key. Symbols are trimmed and uppercased at storage
boundaries.

### `sector_memberships`

A dated membership snapshot keyed by `(as_of_date, sector, symbol)`, with rank
and point-in-time market cap. This separates historical universe composition
from mutable issuer metadata and caps each sector at 100 members.

The current UI reads the latest membership on or before today. Older snapshots
are retained so future releases can show or audit membership changes.

### `bars`

OHLCV observations keyed by `(symbol, timeframe, timestamp)`. Optional trade
count and VWAP and a source label are stored alongside open, high, low, close,
and volume. Repeated history windows upsert the same keys.

### `snapshots`

The newest per-symbol current price, previous close, session open/high/low,
volume, and update time. An older response cannot overwrite a newer snapshot.

### `news` and `news_symbols`

Articles are keyed by provider ID. A separate many-to-many table stores the
ordered related symbols, avoiding duplicated headline content when one article
mentions several companies.

### `favorites`

One row per starred symbol with its creation timestamp. Foreign keys keep
favorites consistent with the company catalog. Favorites persist across
restarts and appear in the dedicated Starred route even if a symbol later
leaves the top-100 universe, provided its company row is retained.

### `sync_checkpoints`

Successful completion times keyed by a textual scope such as `snapshots`,
`history:1Day:2Y`, `history:1Week:all`, their per-symbol child scopes, or the
versioned demo scope. Checkpoints contain no credentials.

## First Live Launch

A live database is prepared in stages:

1. Upsert the embedded SEC-derived catalog. It contains between 100 and 250
   candidates per sector, depending on the number of eligible SEC facts.
   Previously reconciled retention flags survive this bootstrap.
2. Select up to 100 retained initial members per sector. With no cached market
   caps this uses the catalog's descending SEC public-float proxy rank.
3. Fetch Alpaca's active US-equity asset list before requesting snapshots.
   Present catalog candidates are reactivated; missing candidates are removed
   from current membership without deleting their company rows, favorites, or
   cached data. Memberships are then recomputed and current names/exchanges
   are merged without erasing catalog sector, rank, or market-cap metadata.
4. Request current snapshots for all retained sector candidates and the three
   benchmark ETF proxies in configurable batches (100 by default).
5. Where SEC-reported shares are available, calculate an estimated market cap
   from shares times current price. Re-select 100 members per sector by known
   market cap first and proxy rank as fallback, and store a dated snapshot.
6. Start adjusted history requests for those selected 900 companies and three
   benchmark ETF proxies in configurable 50-symbol batches: two years of
   `1Day` bars and all provider-available `1Week` bars.

Other active Alpaca assets can remain searchable without joining a heatmap
sector. If the active-asset request fails, startup reports the provider error
and continues from the last reconciled retention state in the cache.

The UI remains interactive during history population. A tile can be neutral or
marked stale until enough data for its selected range arrives. The Data Status
overlay shows phase, completed/total counts, automatic-refresh cadence, the
latest snapshot-cache checkpoint, status text, and the last provider error.
Opening this overlay with `S` is read-only and does not start a request.

## Incremental History Sync

The bulk cache has two plans: `1Day` bars beginning 731 days before now and
`1Week` bars beginning at the unbounded `ALL` cutoff. For Alpaca, an unbounded
request returns whatever history the account and feed make available rather
than manufacturing data before the provider's coverage.

Each plan records completion per symbol. Before a batch:

- If any member lacks that plan's completion checkpoint or latest-bar
  watermark, the request uses the plan's full initial cutoff. A newly selected
  company therefore cannot inherit a peer's shorter window.
- Otherwise the request begins seven days before the earliest latest-bar
  watermark in the batch.

The seven-day overlap repairs recently adjusted or late bars and makes restart
behavior robust. Primary-key upserts make the overlap idempotent. Each batch is
committed independently, so quitting partway through preserves completed
symbols; a later launch resumes from the stored watermarks. A plan-level
checkpoint is written only after every batch in that plan succeeds.

History requests use `adjustment=all`, ascending order, pagination, and the
configured feed. "Adjusted" is provider-defined and does not guarantee that
every corporate action is represented correctly.

The current retention window is bounded by what synchronization requests, not
by a background pruning job. Repeated overlap upserts do not duplicate rows.

## Current-Day Refresh

The worker refreshes candidate snapshots once on startup and at the configured
cadence, five minutes by default. Each successful refresh can update estimated
market caps and writes that day's top-100 membership. `r` or the Refresh rail
action asks for an immediate snapshot refresh and restarts the cadence timer,
preventing a scheduled refresh immediately afterward. No streaming or
per-trade connection is used. If the prior history job has finished, a
successful refresh also starts another incremental history pass so newly
selected members are backfilled without restarting the application. Demo and
offline modes do not schedule or request remote refreshes.

Snapshots drive `1D` return when price and previous close are present. The UI
falls back to cached bars when snapshot fields are unavailable. A tile is
considered stale when its newest snapshot/bar timestamp is absent or more than
72 hours old; weekends and holidays can therefore look stale after a long
closure, which is an informational hint rather than a feed diagnosis. Stale
ticker labels are underlined while retaining the same contrast-aware foreground
as current labels.

Every broad refresh requests every currently retained candidate and benchmark
proxy. A successful request does not guarantee a new observation for every
symbol: an active but thinly traded security can still carry an older IEX trade
timestamp, and the client deliberately does not replace it with the request
time. The active-asset reconciliation at startup removes inactive catalog
symbols from current membership while preserving their cached history and
favorite state.

## Lazy Detail Sync

Opening a ticker first loads its cached record, then concurrently requests:

- bars for the selected range's preferred timeframe; and
- up to 20 newest ticker-related news records; and
- a current snapshot for price, OHLC, volume, and day return.

Preferred chart timeframes are:

| Range | Preferred request |
| --- | --- |
| `1D` | `5Min` |
| `1W`, `1M` | `1Hour` |
| `3M`, `6M`, `1Y`, `2Y` | `1Day` |
| `5Y`, `10Y`, `ALL` | `1Week` |

While a preferred timeframe is not cached, storage chooses an available
fallback appropriate for that range. Changing the range on a detail view
triggers another lazy request and redraws from whatever is already cached.

News is not globally downloaded for every sector company or benchmark proxy.
This keeps startup and provider usage bounded. Cached headlines remain
available offline.

## Period Calculations And Sorting

For non-day ranges, the baseline is the last close at or before the exact
cutoff, falling back to the first close after it. Return is latest close divided
by baseline minus one. Calendar-day cutoffs mean the number of trading sessions
varies with weekends and holidays. `ALL` uses the earliest bar present in the
provider-backed local cache.

Sort modes operate within each sector:

- Market cap: descending known market cap, then catalog rank.
- Gainers: descending selected-period return.
- Volume: descending latest snapshot or period-bar volume.
- A-Z: ascending ticker symbol.

Missing numeric values sort after present values. Favorites can include
retained companies outside the current universe and are not truncated to 100
by the storage query, although the current grid renders at most 100 at once.

## Search And Retention

Search is a local SQL query over symbol and company name. Exact and prefix
symbol matches rank first, then name prefixes, current-universe status, market
cap, and symbol. The UI requests at most 20 results.

Company rows support `in_universe` and `retained` independently. SEC-derived
sector candidates reported by Alpaca as active are retained so snapshot refresh
can move them into or out of the top 100. A catalog candidate missing from the
active-asset response is marked unretained and removed from current membership,
but its company row, bars, news, and favorite remain intact. A later active
response reactivates that candidate. An updated embedded catalog is still
required to consider an issuer absent from the current candidate set. The
current release does not run automatic garbage collection for old company
rows.

## Offline And Demo Behavior

`--offline` suppresses the provider worker and renders only the selected
database. It does not update freshness timestamps or fetch a search miss.

Demo mode writes simulated records into the selected database and records a
versioned demo checkpoint. It reuses a complete cache only when that checkpoint
matches the current generator. Any recognized older demo checkpoint triggers a
clean regeneration so incompatible historical rows cannot overlap; favorites
whose symbols remain in the new universe are restored. `--reset-demo` clears
every table in the selected database, including favorites and live-provider
data, before regeneration. Because live and demo data share a schema, use
separate paths when switching modes or preserving a valuable live cache.

## Operational Guidance

- Do not edit the database while the app is running unless you understand
  SQLite WAL concurrency and the schema invariants.
- Do not publish a live Alpaca cache. Provider data is not covered by the
  repository's MIT license and ordinary Alpaca terms prohibit redistribution.
- Keep database backups private; favorites and cached news can reveal user
  interests.
- Before reporting corruption, stop the app, preserve the database privately,
  and reproduce with a new `--db` path. Never attach credentials or a populated
  provider cache to a public issue.
- Deleting or moving a database is a manual destructive operation. The app
  will create a new schema on the next launch, but live history must be fetched
  again.
