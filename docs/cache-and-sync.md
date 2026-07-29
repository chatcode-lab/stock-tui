# Cache And Synchronization

`stock-tui` treats SQLite as the source of truth for every rendered market
view. This makes startup immediate after the first population, keeps favorites
available offline, and bounds repeated historical requests.

## Cache Location

Live and offline-cache modes use `market.sqlite3`; demo mode uses
`demo.sqlite3`. Both live in the platform application-data directory selected
by the Rust `directories` crate for organization `chatcode-lab` and
application `stock-tui`.

Typical locations are:

| Platform | Default live application data path |
| --- | --- |
| Linux | `$XDG_DATA_HOME/stock-tui/market.sqlite3`, normally `~/.local/share/stock-tui/market.sqlite3` |
| macOS | `~/Library/Application Support/com.chatcode-lab.stock-tui/market.sqlite3` |
| Windows | The per-user roaming application-data directory for `chatcode-lab/stock-tui`, ending in `data/market.sqlite3` |

Platform conventions can vary. Use `stock-tui --print-config` for the exact
resolved `db_path`, or override it with `--db` / `STOCK_TUI_DB_PATH`.

Releases before 0.1.2 used `market.sqlite3` for both modes. Current releases
also require every live database to carry an explicit provider and market
identity. On the first online launch after upgrading an unstamped legacy
database, the application cannot prove which provider/feed produced its rows,
so it clears observations, news, memberships, checkpoints, and provider
company metadata before synchronization. Starred symbols are preserved as
neutral company rows and rehydrated from the catalog/provider. Demo and live
defaults remain separate.

The application also creates platform config and cache directories. The
configuration file is `<config_dir>/config.toml`; the SQLite database belongs
in the data directory, not the disposable cache directory. Daily diagnostic
logs are written below `<cache_dir>/logs`. A successfully downloaded compact
SEC catalog is stored at `<cache_dir>/catalog/sec_universe.json`. It contains
issuer metadata only, no credentials or provider observations, and can be
recreated from the embedded release catalog plus the public catalog endpoint.

## SQLite Settings

- Schema version is stored in `PRAGMA user_version`.
- Current schema version: 3.
- Journal mode: WAL.
- Foreign keys: enabled on every connection.
- Busy timeout: 30 seconds.
- Batch writes use immediate transactions.
- A binary refuses to open a schema newer than it understands.

Schema 2 is an additive migration. It preserves schema-1 companies, bars,
snapshots, news, favorites, memberships, and checkpoints while adding SEC
ranking-proxy and share-estimate metadata. Schema 3 adds a singleton
`cache_context` record. The migration itself is additive, but the next online
startup deliberately clears legacy provider rows whose provenance cannot be
established. A downgrade to an older binary refuses to open the upgraded
database; it does not attempt a lossy downgrade.

WAL creates adjacent `-wal` and `-shm` files while the database is open. For a
consistent backup, stop `stock-tui` first and copy the database together with
any SQLite sidecars, or use SQLite's online backup tooling.

## Schema

### `companies`

One normalized row per symbol. It stores name, normalized and raw sector,
exchange, industry, optional estimated market cap, the numeric SEC public-float
ranking proxy with source/date/confidence, the share estimate with
source/date/method/confidence, catalog rank, description, current-universe and
retained flags, and metadata update time.

The symbol is the primary key. Symbols are trimmed and uppercased at storage
boundaries.

### `sector_memberships`

A dated membership snapshot keyed by `(as_of_date, sector, symbol)`, with rank,
point-in-time estimated market cap, and the ranking proxy used when that cap is
unavailable. This separates historical universe composition from mutable
issuer metadata and caps each sector at 100 members.

The current UI reads the latest membership on or before today. Older snapshots
are retained so future releases can show or audit membership changes.

### `bars`

OHLCV observations keyed by `(symbol, timeframe, timestamp)`. Optional trade
count and VWAP and a source label are stored alongside open, high, low, close,
and volume. Repeated history windows upsert the same keys.

Some providers return a daily placeholder while a security has no trades. A
bar with zero volume, zero or absent trade count, and identical OHLC prices is
retained here for raw-cache fidelity, but storage does not treat it as a price
observation. It cannot select a timeframe or period endpoint, refresh a
ticker's freshness, extend displayed history coverage, or enter a detail chart.

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

### `cache_context`

One singleton row identifies the only live market dataset allowed in the
database. It stores an opaque provider cache namespace plus the market ID,
symbol namespace, currency, IANA timezone, and regular-session open/close.
Provider namespaces include observation-changing settings such as the selected
feed and base endpoints.

An exact identity match reuses the cache. A mismatch, or an unstamped database
containing legacy provider rows, is reset transactionally before the first UI
render. Bars, snapshots, news, memberships, checkpoints, and provider-derived
company metadata are removed; favorite symbols survive as neutral retained
rows. Listing-exchange text is not a cache boundary: NASDAQ, NYSE, and ARCA
currently share the single `us-equities` market context and calendar. Switching
to another provider market, feed, endpoint, currency, session, or symbol
namespace starts that new context without mixing old rows.

## First Live Launch

A live database is prepared in stages:

1. Build the configured provider and validate the database's `cache_context`.
   Initialize an empty cache, reuse an exact match, or clear incompatible
   provider rows while preserving favorite symbols. Exactly one market context
   is active for the process.
2. Resolve and upsert the newest valid local SEC-derived catalog without
   waiting for the network. It contains between 100 and 250 candidates per
   sector plus dated filing-derived share bases. An ambiguous issuer receives a
   share basis only when its latest filing is unambiguous or matches an exact
   reviewed class and filing-fact policy. Lower-ranked unresolved candidates
   remain available through their numeric public-float proxy, while catalog CI
   requires complete share coverage for the current sector top 100. Previously
   reconciled retention flags survive its upsert.
   When the local copy exceeds 12 hours by default, a background task checks
   the compact R2 catalog and keeps that cadence while the app remains open. A
   valid newer result is cached, applied to SQLite, and queues another provider
   universe reconciliation; network, schema, size, and downgrade failures keep
   the local result.
3. Select up to 100 retained initial members per sector by estimated market cap
   where available and numeric SEC public float otherwise. With no cached
   market caps this is equivalent to descending catalog proxy rank.
4. Fetch the selected asset provider's active instrument list before requesting
   snapshots. Present catalog candidates are reactivated; missing candidates
   are removed from current membership without deleting their company rows,
   favorites, or cached data. Memberships are then recomputed and current
   names/exchanges are merged without erasing catalog sector, proxy,
   share-estimate, or market-cap metadata.
5. Request current snapshots for all retained sector candidates and the three
   benchmark ETF proxies in configurable batches (100 by default). When the
   adapter supports corporate actions, request forward and reverse splits from
   the oldest relevant catalog share date through the refresh date in
   parallel.
6. Prefer a valid market cap supplied with the provider snapshot. Otherwise,
   where a catalog share estimate and corporate-action coverage are available,
   apply intervening split ratios through that snapshot's observation date
   before multiplying price-equivalent common shares by current price. If
   required split coverage fails, leave the local estimate unavailable rather
   than multiply stale shares by a post-split price. Re-select 100 members per
   sector by the resulting estimate or the numeric public-float proxy, and
   store a dated snapshot. The proxy affects selection but never populates the
   market-cap field shown in ticker statistics.
7. Start adjusted history requests for those selected 900 companies and three
   benchmark ETF proxies in configurable 50-symbol batches: two years of
   `1Day` bars and all provider-available `1Week` bars.

Other active provider assets can remain searchable without joining a heatmap
sector. If the active-asset request fails, startup reports the provider error
and continues from the last reconciled retention state in the cache.

The UI remains interactive during history population. A tile can be neutral or
marked stale until enough data for its selected range arrives. Normal chrome
shows the active phase with completed/total counts and percentage. The Data
Status overlay adds automatic-refresh cadence, the latest snapshot-cache
checkpoint, status text, and the last provider error. Opening this overlay with
`S` is read-only and does not start a request.

## Incremental History Sync

The bulk cache has two plans: `1Day` bars beginning 731 days before now and
`1Week` bars beginning at the unbounded `ALL` cutoff. An unbounded request
returns whatever history the selected provider makes available rather than
manufacturing data before its coverage.

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
market caps and writes that day's top-100 membership. Snapshot and available
split requests run together; a split-request error does not discard valid
prices or provider-supplied caps, but it suppresses share-derived caps for that
refresh. Successful per-symbol split coverage, including an empty result, is
cached in memory for up to 24 hours and reused by broad and lazy ticker
refreshes. `r` or the Refresh rail action asks for an immediate snapshot
refresh and restarts the cadence timer, preventing a scheduled refresh
immediately afterward. No streaming or per-trade connection is used. If the
prior history job has finished, a successful refresh also starts another
incremental history pass so newly selected members are backfilled without
restarting the application. Demo and offline modes do not schedule or request
remote refreshes.

Snapshots drive `1D` return when price and previous close are present. The UI
falls back to cached price-observation bars when snapshot fields are
unavailable. A zero-volume, zero-trade flat placeholder cannot refresh the
endpoint timestamp. A tile is considered stale when its newest snapshot or
price-observation timestamp is absent or more than 72 hours old; weekends and
holidays can therefore look stale after a long closure, which is an
informational hint rather than a feed diagnosis. Stale ticker labels are
underlined while retaining the same contrast-aware foreground as current
labels.

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
- a current snapshot for price, OHLC, volume, and day return; and
- forward and reverse splits needed to reconcile a dated local share estimate,
  when the selected provider exposes that capability.

Preferred chart timeframes are:

| Range | Preferred request |
| --- | --- |
| `1D` | `5Min` |
| `1W`, `1M` | `1Hour` |
| `3M`, `6M`, `1Y`, `2Y` | `1Day` |
| `5Y`, `10Y`, `ALL` | `1Week` |

While a preferred timeframe has no price observations, storage chooses an
available fallback appropriate for that range. Changing the range on a detail
view triggers another lazy request and redraws from whatever is already cached.
The detail header summarizes the complete cached price-observation span, and
Statistics shows its first and last dates. A fixed range is visually muted only
when it adds no older observations beyond the next-shorter preset; it remains
selectable, and `ALL` always uses the complete cached span.

News is not globally downloaded for every sector company or benchmark proxy.
This keeps startup and provider usage bounded. Cached headlines remain
available offline.

## Period Calculations And Sorting

The period endpoint is the newest valid price by timestamp between the current
snapshot and the latest cached price-observation bar. Its timestamp also
controls the tile's freshness marker, so an older snapshot cannot override a
newer traded bar, a price-less snapshot cannot make an old price appear
current, and a no-trade placeholder cannot manufacture a fresh close. For
non-day ranges, the baseline is the last observed close at or before the exact
cutoff, falling back to the first observed close after it. For `1D`, a selected
snapshot endpoint uses its previous close when available and otherwise uses
that cached cutoff baseline. Return is endpoint price divided by baseline minus
one. Calendar-day cutoffs mean the number of trading sessions varies with
weekends and holidays. `ALL` uses the earliest price observation present in the
provider-backed local cache.
Heatmap volume is calculated separately from that price endpoint. `1D` uses
latest-session cumulative snapshot volume when the snapshot supplies the
selected price, otherwise it sums cached bars inside the day cutoff. Longer
ranges sum non-negative cached OHLCV volume from the inclusive cutoff through
the newest cached bar no later than now. The aggregation prefers `1Day` bars
through `2Y` and `1Week` bars for `5Y`, `10Y`, and `ALL`, then tries other
cached granularities when the preferred timeframe has no observations. A daily
snapshot is never added to a longer range, avoiding latest-session double
counting; a multi-day total can lag that session until its bar is cached.
Missing range history remains neutral and sorts after known volume.

Ticker price charts add price-only boundary points for the cutoff baseline and
selected endpoint when those values are not already represented by a cached
price-observation bar. `1D` loads a seven-day overlap, selects the newest
exchange-local date containing a regular-session observation, and maps the full
configured open-to-close session to the plot. This keeps the future portion of
an active session blank and lets a weekend launch show Friday rather than an
empty rolling 24-hour window. `1W` loads enough overlap to concatenate the five
newest observed sessions. Other intraday data maps each observed session to an
equal contiguous span, so nights, weekends, and holidays consume no horizontal
space. Daily and weekly bars use ordinal observation spacing.

Consecutive observations retain a direct thin trace. When a long gap occurs
during an open session, the trace carries the last traded price to the next
observation instead of drawing a disconnected hole. Closed-session boundaries
are compressed to the same X position. Flat no-trade placeholders remain
excluded. Volume still comes only from traded provider OHLCV bars, so missing
observations and price-only boundary points cannot fabricate stored or
aggregated volume. For visual continuity only, the renderer repeats the
preceding positive bar's height through later unoccupied columns and the chart
tail using a dim trail. Session grouping uses the active market's IANA timezone;
labels use the user's local timezone.

Sort modes operate within each sector:

- Market cap: descending estimated market cap, or numeric SEC public-float
  proxy when the estimate is unavailable, then catalog rank and symbol.
- Gainers: descending selected-period return.
- Volume: descending cumulative share volume over the selected range; `1D`
  prefers latest-session cumulative snapshot volume.
- A-Z: ascending ticker symbol.

Rows missing both size values sort after rows with either value. Favorites can include
retained companies outside the current universe and are not truncated to 100
by the storage query, although the current grid renders at most 100 at once.

## Search And Retention

Search is a local SQL query over symbol and company name. Exact and prefix
symbol matches rank first, then name prefixes, current-universe status, market
cap, and symbol. The UI requests at most 20 results.

Company rows support `in_universe` and `retained` independently. SEC-derived
sector candidates reported by the selected asset provider as active are retained
so snapshot refresh can move them into or out of the top 100. A catalog
candidate missing from the active-asset response is marked unretained and
removed from current membership, but its company row, bars, news, and favorite
remain intact. A later active response reactivates that candidate. A newly
published remote catalog or a newer embedded release catalog is still required
to consider an issuer absent from the current candidate set. The current
release does not run automatic garbage collection for old company rows.

## Offline And Demo Behavior

`--offline` suppresses the provider worker and renders only the selected
database. It does not request a catalog update, update market-data freshness
timestamps, or fetch a search miss. The runtime can still seed issuer
identities from a valid local catalog cache or its embedded release fallback;
missing market observations remain empty. A stamped database reuses its stored
market context for chart sessions. An unstamped legacy database is left
untouched in offline mode and falls back to the current US-equities session
profile because no provider is available to establish a safer identity.

Demo mode is entered with `--demo` or by explicitly choosing `d` during
onboarding; missing credentials alone do not select it. A normal online launch
completes credential onboarding before opening either database. Demo mode
writes simulated records into the selected database and records a versioned
demo checkpoint. It reuses a complete cache only when that checkpoint matches
the current generator. Any recognized older demo checkpoint triggers a clean
regeneration so incompatible historical rows cannot overlap; favorites whose
symbols remain in the new universe are restored.
`--reset-demo` clears every table in the selected database, including favorites
and live-provider data, before regeneration. Because live and demo data share a
schema, use separate paths when switching modes or preserving a valuable live
cache.

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
