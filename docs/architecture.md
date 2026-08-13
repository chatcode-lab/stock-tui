# Architecture

This document describes the current native client architecture and the design
boundaries contributors should preserve.

## Goals

`stock-tui` is optimized for three properties:

1. The useful screen appears from local data without waiting for a network
   bootstrap.
2. A wide universe remains scannable in a terminal without losing mouse or
   keyboard accessibility.
3. Provider-specific payloads, credentials, and licensing rules do not leak
   into the domain, storage, or rendering layers.

The application is read-only and contains no order-entry path.

## Process Model

```text
terminal events ----> app commands ----> runtime ----> local SQLite
       |                                    |              ^
       v                                    v              |
  Ratatui render <---- UI state <---- sync events <---- provider worker
                                             selected adapter / demo generator
```

There is one foreground terminal event loop and, in live mode, one asynchronous
provider worker. SQLite is the handoff boundary: the worker normalizes and
upserts remote results, then emits a small invalidation event; the UI reloads
the relevant cached view. The renderer never performs HTTP requests.

## Modules

| Module | Responsibility |
| --- | --- |
| `cli` | Parses command-line flags and environment-backed overrides with Clap. |
| `config` | Resolves project directories, `.env`, TOML, environment, defaults, and redacted credentials; it updates onboarding credentials in TOML with owner-only Unix permissions. |
| `credentials` | Reads the legacy dotenv credential file as a lower-precedence compatibility fallback. |
| `onboarding` | Offers open/copy/skip registration actions or demo mode, collects hidden input, validates credentials, and starts the selected mode. |
| `logging` | Writes non-ANSI daily tracing logs below the platform cache directory. |
| `domain` | Defines sectors, date ranges, sort modes, companies, bars, snapshots, stock splits, news, tiles, and sync state. |
| `benchmarks` | Defines the labeled ETF proxies displayed beneath the sector overview. |
| `universe` | Validates and resolves the remote, cached, or embedded versioned issuer catalog used to seed the nine sector memberships. |
| `providers` | Defines independent asset, market-data, corporate-action, and news capabilities plus concrete provider adapters. |
| `storage` | Owns SQLite migrations, transactions, search, favorites, period metrics, and detail queries. |
| `sync` | Schedules snapshot and corporate-action refresh, incremental history, asset metadata, and lazy ticker/news requests. |
| `demo` | Generates deterministic simulated data for all screens and date ranges. |
| `app` | Converts keyboard, paste, and mouse events into UI transitions and runtime commands. |
| `ui` | Calculates responsive layout, registers mouse hit targets, and renders heatmaps, overlays, and charts. |
| `runtime` | Wires terminal input, render ticks, storage, commands, refresh cadence, and worker events together. |
| `terminal` | Formats OSC 8 links and OSC 52 clipboard writes, enters raw alternate-screen mode, requests text-based SGR mouse reports, and performs ordered, input-draining restoration on exit or panic. |

## Startup Paths

Settings and project directories are resolved before the alternate screen is
entered. A normal online launch validates the selected provider's
configuration. Alpaca launches validate configured credentials and complete
onboarding when needed; `stock-api` launches do not enter Alpaca onboarding.
Storage then opens, enables foreign keys and WAL, and applies forward schema
migrations.

### Demo Mode

Demo mode is selected by `--demo` or the first-run onboarding prompt. The
onboarding choice affects only that launch and switches the normal default
database path to `demo.sqlite3`; an explicit database override is preserved.
The runtime opens the selected SQLite file and seeds it on a blocking worker if
it does not already contain a complete demo data set.

The generator selects the first 100 ranked identities in each of the nine
sectors from the embedded SEC catalog. It then creates simulated rankings,
snapshots, two clearly marked simulated headlines per company, and `5Min`,
`1Hour`, `1Day`, and `1Week` bars sufficient for every range through `10Y` and
the complete generated demo history used by `ALL`. Issuer identity and exchange
associations come from the catalog; every displayed market value is
deterministic demo data rather than a factual quote.

### Live Mode

With Alpaca selected, live mode requires a complete pair from the environment,
a working-directory dotenv file, `[providers.alpaca]` in the platform
`config.toml`, or the legacy credential-file fallback. A missing pair starts
onboarding; it does not manufacture live data or silently switch to demo.
`stock-api` uses its own endpoint and optional token configuration and does not
require Alpaca credentials.

The runtime first resolves the newest valid local SEC-derived catalog and
renders without waiting for a network request. Unless `--offline` is set, a
background task rechecks the compact gzip-served JSON from R2 at startup and
after each configured cache interval while the app remains open. It applies
the same schema, rank, identifier, provenance, safe-text, and size validation
used for the embedded catalog, rejects downgrades, and atomically caches a
valid result. Network, format, size, and freshness failures preserve the newest
valid cached or embedded copy.

The selected local catalog is upserted and supplies 100 dated members per
sector. Newly generated catalogs also carry the SEC SIC taxonomy's short
industry label and a bounded CC0 company profile. Profiles use SEC CIK first,
then a conservative canonical-entity lookup with current ticker/exchange and
business evidence when the CIK result is missing or administrative. The roomy
ticker-detail layout consumes that context without another client-side profile
service; older schema-v2 catalogs remain valid and use a numeric SIC fallback.
Catalog share bases come from
recent unambiguous filing facts or exact-signature reviewed policies for
economically ambiguous issuer structures. Catalog publication fails if any
candidate currently ranked in a sector top 100 lacks a share basis. A cached
market cap is carried forward only when its share estimate and provenance still
match the catalog. Candidates without a calculated market cap compete using
their numeric SEC public-float proxy. A valid background update is applied to
SQLite and queues a provider universe reconciliation. The runtime loads cached
tiles and starts the selected provider worker unless `--offline` is set.

The worker initially:

1. Reconciles the candidate catalog against the adapter's active assets
   and recomputes memberships. Active catalog candidates are retained or
   reactivated; missing candidates leave the current universe while their
   rows, cached data, and favorites remain stored.
2. Fetches snapshots for retained candidates in batches and, when supported,
   the forward and reverse splits needed to reconcile dated catalog shares.
3. Prefers a valid provider snapshot cap. Otherwise it applies intervening
   split ratios to the catalog's price-equivalent common-share estimate before
   multiplying by current price. A required split-coverage failure suppresses
   the local estimate. It then writes a new top-100 membership for each sector.
   Selection compares the resulting cap with numeric SEC public float for
   proxy-only candidates; catalog rank and symbol provide stable ties.
4. Starts adjusted two-year daily-bar and all-provider-available weekly-bar
   backfills for the selected 900 members and three benchmark ETF proxies in
   the background.

It then accepts distinct manual and timed refresh commands. The timed command
requests snapshots only for current sector members, benchmark proxies, and
explicit favorites; it neither scans the broader retained candidate pool nor
starts history. When the terminal multiplexer forwards focus events, the
runtime pauses that timer on `FocusLost` and starts a fresh interval on
`FocusGained`. A manual refresh requests broad
candidate snapshots and starts incremental history when the history worker is
idle. A catalog update first reconciles assets, then uses the same broad
snapshot and history policy. This confines discovery of potential new top-100
entrants to startup, manual refresh, and catalog reconciliation.

Ticker-detail requests remain independent. Opening a ticker reads cached
detail immediately and requests a current snapshot, the preferred chart
timeframe, and up to 20 related news items in parallel.

See [Cache and Sync](cache-and-sync.md) for watermarks and failure behavior.

## UI State And Routing

The UI has four routes:

- `Overview`: nine sector panels in a fixed 3x3 order plus a selectable
  `SPY`/`DIA`/`QQQ` benchmark-proxy strip.
- `Sector`: up to 100 companies in an adaptive grid.
- `Ticker`: a tinted detail view for one cached company.
- `Favorites`: the persisted starred-company subset.

Search, ordering, keyboard help, and sync status are overlays rather than routes. This
keeps the underlying market context intact while an overlay is open.

Each frame clears and rebuilds a list of rectangular hit targets. Mouse input
is resolved against that list in reverse paint order, so modal controls win
over content beneath them. The same target actions feed the same state
transitions used by keyboard input. Overview hit targets cover whole sector
panels; sector and news-row hover moves the persistent selection used by the
keyboard. Returning from ticker detail restores the originating sector or
Favorites selection.

Overview sector and benchmark focus are mutually exclusive. Moving or hovering
onto a benchmark preserves the sector cursor for later navigation but
suppresses every sector highlight; focusing any sector clears benchmark
emphasis.

`Backspace` and `Space` cycle backward and forward through sibling views with
wraparound. A sector route follows the fixed `Sector::ALL` order and retains
the selected tile position when the destination has that many entries. Ticker
detail follows the exact displayed order saved by its originating sector or
Favorites route; benchmark details follow `SPY`, `DIA`, then `QQQ`. The header
derives its one-based position and total from that same list, so its rank
always matches the active sort. `Esc` alone goes up one route level.

Sector shortcuts use a terminal-safe two-key chord: `g` arms the chord and the
next `c/s/h/e/t/f/i/m/u` selects the corresponding sector. Escape, Backspace,
mouse input, overlays, or one non-sector key cancel the pending prefix.
Escape and Backspace stop after cancellation; another non-sector key is
handled normally. Alt/Meta variants remain optional compatibility shortcuts
for terminals that transmit those modifiers.

## Responsive Rendering

The minimum coherent viewport is 60x20. Smaller viewports render only a resize
message. At exactly 60x20 the secondary header status row collapses so every
overview sector retains five paired heatmap rows. Larger viewports reserve a
two-row header; every supported size keeps a right action rail and a one-row
status footer.

Full mode begins at 120x36. It uses a 15-column rail and a split detail view.
Compact mode uses a 12-column rail and replaces the detail split with Chart,
Statistics, and News tabs.

The overview always has three columns and three rows. Panels and ticker cells
use uniform dimensions; indivisible terminal rows and columns become centered
outer padding. A sector panel with ten body rows draws its full 10x10 tile
matrix. A shorter panel draws two ticker colors per terminal cell with the
upper-half block character, retaining all 100 signals in five rows. Grid maps
rank directly to row-major cells; Spiral maps rank center-out clockwise. The
compact renderer applies the inverse mapping before combining logical rows, so
both presentations keep the same order at every supported height. Sector
detail uses ten columns when possible and otherwise selects between three and
ten columns from the available width. The three benchmark-proxy footer cells
reuse the overview's centered three-column geometry and stop at the content
pane rather than extending beneath the action rail.

Charts use a market-time timeline instead of raw wall-clock gaps. `1D` selects
the newest regular session with an observation and maps its full configured
open-to-close interval, leaving an active session's future tail blank. Intraday
multi-session ranges concatenate observed exchange-local sessions and give
closed periods zero horizontal width; `1W` keeps the latest five. Daily and
weekly histories use ordinal observation spacing. Normal adjacent observations
retain a direct line, while a long in-session gap carries the prior traded
price to the next observation. Volume shares the same mapping. Its presentation
layer repeats the preceding positive bar height through unoccupied columns only
as far as the rendered price trace using a dim color, so an ongoing session's
future tail stays blank. Observed volume, aggregation, statistics, and scaling
remain unchanged.

A Braille canvas renders the thin price trace over a per-cell RGB area fill,
with price and range-aware date scales. The fill samples the same two
horizontal Braille subcells as the trace and uses fractional edge coverage plus
a short exterior fade to soften its cell-resolution boundary. Horizontal
reference guides use the terminal font's middle-dot glyph instead of full-width
Braille runs, preventing fallback-font advance errors from accumulating across
browser-hosted terminal rows. The trace replaces guide dots at intersections.
Price labels are painted over the plot after the chart, using an opaque panel
background for legibility. Hover or keyboard selection then replaces one fixed
terminal column with centered middle dots; its price intersection uses one
inverse cyan cell with the same cursor glyph. When space permits, an exact
price is placed beside that intersection and its date or time beside the X
axis, on the roomier side of the cursor.

Ticker detail derives complete cached-history coverage from the earliest and
latest price observations across timeframes. The header exposes that span. A
fixed range control is muted when it adds no older cached interval
beyond the next-shorter preset, without changing its hit target or shortcut;
`ALL` remains the unbounded cached-history choice.

A responsive 4-7-row volume histogram uses uniform-color lower-block caps for
eighth-cell height precision. Fully occupied cells use background color instead
of a full-block glyph, avoiding line-height seams through the solid portions.
Observed bars use the primary accent; repeated-height trail columns use a
panel-weighted accent and remain visually subdued when the cursor crosses them.

## Heatmap Semantics

Storage selects the newest valid price endpoint by timestamp from the current
snapshot and the latest cached price-observation bar. A zero-volume bar with
zero or absent trades and identical OHLC prices remains in raw storage but
cannot select a timeframe, endpoint, freshness timestamp, coverage boundary, or
chart point. For `1D`, a selected snapshot endpoint uses its previous close
when available; otherwise, and for longer ranges, the baseline is the nearest
observed close at the exact period cutoff. The fallback timeframe order is
range-specific, so the UI remains useful while finer history is still loading.
Timeframe selection probes the indexed `(symbol, timeframe)` key instead of
enumerating distinct timeframes across the full bars history on every range
change. Detail price plots reconcile the cutoff baseline and selected current
endpoint with the traded close series. The volume plot continues to use only
traded provider OHLCV bars, so those price-only boundary points cannot
fabricate volume.

Heatmap volume is deliberately independent of the price/freshness endpoint.
`1D` uses latest-session cumulative snapshot volume when that snapshot is the
selected price endpoint, with the cached period sum as fallback. Longer ranges
sum one cached OHLCV timeframe from the inclusive cutoff through the newest
cached bar no later than now: daily bars are preferred through `2Y`, and weekly
bars are preferred for `5Y`, `10Y`, and `ALL`. Other granularities are
considered only when the preferred aggregate is unavailable. The client never
adds a daily snapshot to a multi-day bar total, which would double-count the
latest session; a total can therefore lag that session until a corresponding
bar is cached. Missing range history remains neutral.

Except when ordering by Volume, the color extent is the 90th percentile of
absolute returns across loaded tiles, with a 0.5% floor for `1D` and a 1% floor
for longer ranges. Values outside that extent saturate at the brightest
red/green palette endpoint. Volume ordering builds an independent log scale
for each sector from its 10th through 90th percentile. In color themes, hue
distinguishes sectors and intensity identifies selected-range cumulative volume
within that sector; monochrome intentionally retains only the intensity signal.
Sector headers always report return and weight each available return by
estimated market cap, falling back to numeric SEC public float for proxy-only
issuers and equal weight only when neither size is available.

## Storage Boundary

SQLite is authoritative for UI-visible data, including favorites and search.
Remote payloads are never rendered directly. Writes use transactions and
idempotent primary keys; newer snapshots replace older ones. Dated sector
memberships preserve the universe snapshot independently of current company
metadata.

The current schema is described in [Cache and Sync](cache-and-sync.md). Schema
changes must be additive migrations, update `PRAGMA user_version`, and include
round-trip tests. A binary must reject a database created by a newer schema it
does not understand.

## Provider Boundary

`AssetProvider` supplies active/searchable instruments.
`MarketDataProvider` supplies snapshots, bars, and provider-specific historical
availability cutoffs. `CorporateActionsProvider` optionally supplies dated
forward and reverse split ratios; `NewsProvider` is independently optional.
`ProviderSet` is the runtime facade over those capabilities, so `sync` contains
no settings, credential, or Alpaca dependency.

`ProviderSet` also owns a stable cache namespace and a normalized
`MarketContext`: market/calendar ID, symbol namespace, currency, IANA timezone,
and regular session. Startup stores that identity in SQLite and clears
unattributable provider rows before rendering when the namespace or market
semantics change. Listing exchange is issuer metadata, not a cache boundary;
multiple exchanges can share one market context. The current adapters declare
one US-equities context for NASDAQ, NYSE, and ARCA instruments.

`ProviderKind` selects a compiled adapter at the configuration/runtime edge.
Alpaca authentication, pagination, feed selection, response shapes, retry
headers, and error redaction stay inside the Alpaca adapter. Future providers
must add their own onboarding/credential path rather than reusing Alpaca
assumptions.

The `stock-api` adapter demonstrates an isolated optional-authentication path.
It maps the versioned [Stock API HTTP Contract](stock-api-contract.md) into the
same domain types, validates HTTPS/loopback transport and bounded responses,
can omit the news capability, and owns its bearer header. The token may come
from the environment or private TOML configuration, but it never enters the
provider-neutral interfaces.
Alpaca and other adapters never receive that token. Its separately operated
service remains responsible for data provenance and redistribution rights.

Adding a provider requires more than implementing HTTP calls. Contributors
must document provenance, timestamp and adjustment semantics, entitlements,
cache retention, attribution, and redistribution restrictions. See
[Data Providers](data-providers.md).

## Failure Model

- Normal terminal shutdown has two phases. Bracketed paste, focus, and SGR
  mouse reporting are disabled and flushed while input is still raw; the
  `EventStream` is then dropped and background tasks are canceled or joined.
  Pending Crossterm events are discarded until input has been quiet for 40 ms,
  capped at 200 ms. Unix builds flush unread terminal bytes and Windows builds
  drain queued console input before leaving the alternate screen, showing the
  cursor, and restoring raw mode last.
- Setup failures, ordinary errors, and `Drop` attempt every cleanup operation
  while retaining the first error. The panic hook uses the same safe ordering
  with an immediate input flush instead of the quiet-period wait.
- HTTP has a 20-second request timeout and at most three retries after the
  initial attempt.
- Timeouts, `408`, `429`, and selected `5xx` responses use bounded exponential
  backoff; `Retry-After` is honored up to 30 seconds.
- Provider errors update the status/sync overlay but do not delete cached data.
- A failed corporate-action lookup leaves a share-derived market cap unavailable
  unless the snapshot supplies a valid provider cap; it never combines a stale
  catalog share count with a post-split price.
- A new or changed multi-class signature or accession-scoped economic fact
  remains unresolved until its SEC-cited policy is reviewed; the catalog build
  fails when this affects a current sector top-100 candidate.
- Catalog refresh errors preserve the last valid local catalog and do not block
  startup; an oversized, malformed, unsupported, or older catalog is ignored.
- Each history batch is independently upserted, so a later run resumes from
  per-symbol checkpoints and cached watermarks rather than restarting each
  complete history window.
- Normal shutdown cooperatively cancels demo generation and history
  transactions, joins demo seeding, gives the provider worker a short bounded
  grace period to finish, and then aborts and joins outstanding provider or
  catalog work before the shell becomes interactive.
- Offline mode never creates a provider worker.

## Security And Privacy

Credentials enter through hidden onboarding input, environment variables
(including a local dotenv file), `[providers.alpaca]` in `config.toml`, or a
legacy credential file, and are held in secret wrappers. Onboarding preserves
existing TOML comments and settings when it stores the validated pair in
`<config_dir>/config.toml`; Unix permissions are forced to `0600` on write.
Debug output and provider errors redact known credential values before
truncation. The `stock-api` adapter also removes terminal control characters
from remote errors before they reach status or logs. Credentials are not stored
in SQLite, logs, or rendering buffers.

Daily tracing logs are written under `<cache_dir>/logs`. Logs are designed not
to contain credentials, but provider errors and user activity can still be
sensitive and should not be posted without review.

News URLs are untrusted remote content. They are handed to the operating
system's default browser only after explicit activation. If browser launch
fails, the same URL is Base64-encoded into an OSC 52 terminal clipboard
sequence; no shell command interpolates it. The local cache may still reveal a
user's searches indirectly through retained companies, news, and favorites;
protect it like other personal application data.

## Hosted Data Boundary

The default production model is deliberately bring-your-own-key. The project
has no public shared-key price/news backend, and an ordinary personal Alpaca key
must never be used to create one. A separately operated `stock-api` endpoint is
restricted to explicitly authorized development tests and is not a licensed
public market-data service. A future public provider service would need a
distinct client contract and explicit redistribution rights for every served
market-data and news field, plus authentication, abuse controls, freshness
metadata, and documented retention/deletion rules. See
[Requesting Public-Display Permission](data-providers.md#requesting-public-display-permission).

The public `stock.chatcode.dev` object is not such a backend. It serves only a
compact catalog derived from SEC issuer and filing data; prices, bars, volume,
and news continue to come directly from the selected user-authorized provider.
