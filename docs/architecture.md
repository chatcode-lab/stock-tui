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
                                                   Alpaca / demo generator
```

There is one foreground terminal event loop and, in live mode, one asynchronous
provider worker. SQLite is the handoff boundary: the worker normalizes and
upserts remote results, then emits a small invalidation event; the UI reloads
the relevant cached view. The renderer never performs HTTP requests.

## Modules

| Module | Responsibility |
| --- | --- |
| `cli` | Parses command-line flags and environment-backed overrides with Clap. |
| `config` | Resolves project directories, `.env`, TOML, environment, defaults, and redacted credentials. |
| `logging` | Writes non-ANSI daily tracing logs below the platform cache directory. |
| `domain` | Defines sectors, date ranges, sort modes, companies, bars, snapshots, news, tiles, and sync state. |
| `benchmarks` | Defines the labeled ETF proxies displayed beneath the sector overview. |
| `universe` | Loads the versioned issuer catalog used to seed the nine sector memberships. |
| `providers` | Defines provider traits and translates authenticated Alpaca responses into domain records. |
| `storage` | Owns SQLite migrations, transactions, search, favorites, period metrics, and detail queries. |
| `sync` | Schedules snapshot refresh, incremental history, asset metadata, and lazy ticker/news requests. |
| `demo` | Generates deterministic simulated data for all screens and date ranges. |
| `app` | Converts keyboard, paste, and mouse events into UI transitions and runtime commands. |
| `ui` | Calculates responsive layout, registers mouse hit targets, and renders heatmaps, overlays, and charts. |
| `runtime` | Wires terminal input, render ticks, storage, commands, refresh cadence, and worker events together. |
| `terminal` | Enters raw alternate-screen mode, requests text-based SGR mouse reports, and restores the terminal on exit or panic. |

## Startup Paths

Settings and project directories are resolved before the alternate screen is
entered. Storage opens next, enables foreign keys and WAL, and applies forward
schema migrations.

### Demo Mode

Demo mode is selected by `--demo` or by the absence of both Alpaca credential
variables. The runtime opens the selected SQLite file and seeds it on a
blocking worker if it does not already contain a complete demo data set.

The generator selects the first 100 ranked identities in each of the nine
sectors from the embedded SEC catalog. It then creates simulated rankings,
snapshots, two clearly marked simulated headlines per company, and `5Min`,
`1Hour`, `1Day`, and `1Week` bars sufficient for every range through `10Y` and
the complete generated demo history used by `ALL`. Issuer identity and exchange
associations come from the catalog; every displayed market value is
deterministic demo data rather than a factual quote.

### Live Mode

Live mode requires both Alpaca credential variables. The runtime upserts the
versioned SEC-derived candidate catalog, carries forward any cached market caps
and shares, and selects 100 dated members per sector. Candidates without a
calculated market cap initially fall back to their catalog public-float proxy
rank. It then loads cached tiles and starts the provider worker unless
`--offline` is set.

The worker initially:

1. Reconciles the candidate catalog against Alpaca's active US-equity assets
   and recomputes memberships. Active catalog candidates are retained or
   reactivated; missing candidates leave the current universe while their
   rows, cached data, and favorites remain stored.
2. Fetches snapshots for retained candidates in batches.
3. Estimates market cap as current price times SEC-reported shares where both
   exist, then writes a new top-100 membership for each sector. Catalog proxy
   rank breaks ties and covers candidates without usable shares.
4. Starts adjusted two-year daily-bar and all-provider-available weekly-bar
   backfills for the selected 900 members and three benchmark ETF proxies in
   the background.

It then accepts manual or timed snapshot refresh commands and ticker-detail
requests. Opening a ticker reads cached detail immediately and requests a
current snapshot, the preferred chart timeframe, and up to 20 related news
items in parallel.

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

`p` and `n` cycle through sibling views with wraparound. A sector route follows
the fixed `Sector::ALL` order and retains the selected tile position when the
destination has that many entries. Ticker detail follows the exact displayed
order saved by its originating sector or Favorites route; benchmark details
follow `SPY`, `DIA`, then `QQQ`. The header derives its one-based position and
total from that same list, so its rank always matches the active sort.

Sector shortcuts use a terminal-safe two-key chord: `g` arms the chord and the
next `c/s/h/e/t/f/i/m/u` selects the corresponding sector. Escape, Backspace,
mouse input, overlays, or one non-sector key cancel the pending prefix; a
non-sector key is then handled normally. Alt/Meta variants remain optional
compatibility shortcuts for terminals that transmit those modifiers.

## Responsive Rendering

The minimum coherent viewport is 60x20. Smaller viewports render only a resize
message. At 60x20 and above, the layout reserves a two-row header, a right
action rail, and a one-row status footer.

Full mode begins at 120x36. It uses a 15-column rail and a split detail view.
Compact mode uses a 12-column rail and replaces the detail split with Chart,
Statistics, and News tabs.

The overview always has three columns and three rows. Panels and ticker cells
use uniform dimensions; indivisible terminal rows and columns become centered
outer padding. A sector panel with ten body rows draws its full 10x10 tile
matrix. A shorter panel draws two ticker colors per terminal cell with the
upper-half block character, retaining all 100 signals in five rows. Sector
detail uses ten columns when possible and otherwise selects between three and
ten columns from the available width. The three benchmark-proxy footer cells
reuse the overview's centered three-column geometry and stop at the content
pane rather than extending beneath the action rail.

Charts sample cached bars to terminal resolution while preserving the first
and last point. A Braille canvas renders the thin price trace over a per-cell
RGB area fill, with price and range-aware date scales. The fill samples the same
two horizontal Braille subcells as the trace and uses fractional edge coverage
plus a short exterior fade to soften its cell-resolution boundary. Horizontal
reference guides use the terminal font's middle-dot glyph instead of full-width
Braille runs, preventing fallback-font advance errors from accumulating across
browser-hosted terminal rows. The trace replaces guide dots at intersections.
Price labels are painted over the plot after the chart, using an opaque panel
background for legibility. Hover or keyboard selection then replaces one fixed
left or right Braille subcolumn; its price intersection uses one cyan cell with
a dark version of the same cursor glyph.

A responsive 4-7-row volume histogram uses uniform-color lower-block caps for
eighth-cell height precision. Fully occupied cells use background color instead
of a full-block glyph, avoiding line-height seams through the solid portions.

## Heatmap Semantics

For `1D`, the preferred return is snapshot price divided by previous close.
For longer ranges, storage chooses the best cached timeframe and compares the
latest close to the nearest close at the period cutoff. The fallback order is
range-specific, so the UI remains useful while finer history is still loading.
Timeframe selection probes the indexed `(symbol, timeframe)` key in fallback
order instead of enumerating distinct timeframes across the full bars history
on every range change.

The color extent is the 90th percentile of absolute returns across loaded
tiles, with a 0.5% floor for `1D` and a 1% floor for longer ranges. Values
outside that extent saturate at the brightest palette endpoint. Sector headers
show a market-cap-weighted mean when market capitalization is available and
equal weighting otherwise.

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

`MarketDataProvider` covers assets, bars, and snapshots. `NewsProvider` covers
ticker news. Alpaca authentication, pagination, feed selection, response
shapes, retry headers, and error redaction stay inside the Alpaca adapter.

Adding a provider requires more than implementing HTTP calls. Contributors
must document provenance, timestamp and adjustment semantics, entitlements,
cache retention, attribution, and redistribution restrictions. See
[Data Providers](data-providers.md).

## Failure Model

- The terminal is restored by normal drop and by a panic hook.
- HTTP has a 20-second request timeout and at most three retries after the
  initial attempt.
- Timeouts, `408`, `429`, and selected `5xx` responses use bounded exponential
  backoff; `Retry-After` is honored up to 30 seconds.
- Provider errors update the status/sync overlay but do not delete cached data.
- Each history batch is independently upserted, so a later run resumes from
  per-symbol checkpoints and cached watermarks rather than restarting each
  complete history window.
- Normal shutdown gives the provider worker a bounded grace period to finish
  current cache work before outstanding network tasks are aborted.
- Offline mode never creates a provider worker.

## Security And Privacy

Credentials enter only through environment variables (including a local
dotenv file) and are held in secret wrappers. Debug output and provider errors
redact known credential values. They are not stored in SQLite or TOML.

Daily tracing logs are written under `<cache_dir>/logs`. Logs are designed not
to contain credentials, but provider errors and user activity can still be
sensitive and should not be posted without review.

News URLs are untrusted remote content. They are handed to the operating
system's default browser only after explicit activation. If browser launch
fails, the same URL is Base64-encoded into an OSC 52 terminal clipboard
sequence; no shell command interpolates it. The local cache may still reveal a
user's searches indirectly through retained companies, news, and favorites;
protect it like other personal application data.

## Planned Backend Boundary

A lightweight no-key backend is a future deployment option, not a hidden mode
in the current binary. It should implement a distinct client provider contract
and must not expose or proxy an ordinary personal Alpaca key. Before such a
service can ship, its operator needs explicit licenses for redistribution of
every served market-data and news field, plus authentication, abuse controls,
freshness metadata, and documented retention/deletion rules.
