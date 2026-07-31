# Changelog

All notable changes to `stock-tui` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html) after its initial
pre-1.0 releases.

## [Unreleased]

### Changed

- Removed the redundant cached-history row from ticker Statistics; the same
  coverage remains visible in the detail header.

### Fixed

- Size related-news rows to their wrapped headlines and reserve a dedicated
  metadata line, keeping every visible date/source attached to its article
  without blank separator rows, selection-dependent wrapping, or off-screen
  keyboard focus.

## [0.2.7] - 2026-07-30

### Changed

- Enrich company profiles with bounded Wikidata industry, product, and service
  facts, and re-evaluate durable cached profiles when the matching algorithm
  changes.
- Apply downloaded catalog metadata and all sector memberships atomically, then
  refresh an open ticker detail immediately before reconciling provider assets.

### Fixed

- Reject incorporation, jurisdiction, promotional, and other non-business
  boilerplate when building company introductions, and require conservative
  identity evidence before accepting a normalized-name Wikidata result.
- Prevent in-flight universe and snapshot requests from overwriting newer
  catalog descriptions, industries, membership state, or unrelated cached
  market caps.

## [0.2.6] - 2026-07-30

### Added

- Enriched roomy ticker-detail company context with concise CC0 Wikidata
  descriptions and industry labels matched strictly by SEC CIK.
- Retained each profile's Wikidata source URL in compact catalogs and
  CC0/source provenance in audit catalogs and the durable builder snapshot.

### Changed

- Replaced the generic "listed and classified" sentence with a
  company-specific profile plus explicit listing/SIC context, or a readable
  classification fallback when no unambiguous profile is available.
- Persisted positive and empty company-profile lookups per SEC CIK, limiting
  routine catalog builds to new or materially renamed issuers and reserving a
  full refresh for the monthly or explicitly requested catalog run. Stable and
  content-addressed R2 snapshots preserve this builder state independently of
  GitHub Actions cache retention.
- Added catalog publication and release gates for safe, coupled company-profile
  fields and minimum total and per-sector enrichment coverage.

### Fixed

- Restored `stock-api` market-cap estimates for dotted share-class symbols such
  as `BF.A` and `BRK.B` by normalizing hyphenated SEC catalog identifiers before
  lookup and persistence.
- Preserved `stock-api` market-cap source, date, method, and confidence fields
  when compact catalogs store fact provenance in nested records.

## [0.2.5] - 2026-07-30

### Added

- Added concise company context to roomy ticker-detail layouts, backed by
  official SEC SIC industry labels in freshly generated catalogs and a clean
  exchange/SIC fallback for older schema-v2 catalogs.

### Changed

- Require every newly published and release-embedded catalog company to carry
  a safe, nonempty SIC industry label.

### Fixed

- Stop dim synthetic volume trails at the final rendered price column so the
  future portion of an ongoing session remains blank.

## [0.2.4] - 2026-07-29

### Changed

- Fill unoccupied ticker-detail volume columns after the first positive bar
  with a dim same-height visual trail, including the remaining chart tail,
  without changing observed volume, statistics, aggregation, or scale.

## [0.2.3] - 2026-07-29

### Added

- Added a normalized market context with calendar ID, symbol namespace,
  currency, IANA timezone, and regular-session bounds, including DST-aware US
  equity sessions.
- Added SQLite schema 3 cache identities so provider datasets, endpoints,
  feeds, and market contexts are validated before cached rows are rendered.
- Added latest-filing XBRL cover-share extraction and an SEC-cited declarative
  policy registry for multi-class, tracking-stock, Up-C, partnership, and SPAC
  capitalization structures.
- Added catalog share-coverage diagnostics and a publication gate that rejects
  unresolved current sector top-100 candidates.

### Changed

- Render `1D` across the latest observed full regular session, concatenate the
  latest five observed sessions for `1W`, compress closed time in intraday
  multi-session charts, and use ordinal spacing for daily/weekly histories.
- Extend lazy intraday history requests enough to recover the last completed
  session and complete session-day boundaries around weekends and holidays.
- Reject share facts older than 550 days and fail closed when a reviewed
  filing's exact class-member or accession-scoped economic-fact signature
  changes.

### Fixed

- Prevent bars, snapshots, news, memberships, and sync checkpoints from
  silently mixing after a provider, endpoint, feed, or market-context change.
  Incompatible rows are cleared transactionally while favorite symbols remain
  available for rehydration.
- Keep long no-trade intervals connected with the prior traded price without
  manufacturing volume, while retaining a direct thin trace between normal
  adjacent observations.
- Keep `GOOG` as Alphabet's concise catalog member when upgrading a cache that
  previously selected `GOOGL`; the Class A ticker remains independently
  searchable and favoritable rather than being treated as a price alias.
- Keep the chart cursor visible over sparse interior columns by repeating the
  preceding observation at the hovered column.
- Mute a ticker range only when it adds no cached interval beyond the
  next-shorter preset, so partial `10Y` coverage remains visibly useful.
- Derive DELL's market cap from its current price and a reviewed SEC aggregate
  of equal-economic Class A, B, and C shares instead of leaving the value
  unavailable.
- Restore share-derived market-cap estimates for the remaining 44 previously
  unresolved catalog issuers after Dell, including the listed consumer, energy,
  financial, industrial, and services cases, while retaining confidence and
  policy provenance.
- Correct reviewed capitalization scopes for ERIE, PJT, MC, and HLNE using
  filing-reported conversion ratios or economic-unit facts, and version Visa's
  conversion policy for its newly reported Class B-3 shares.

## [0.2.2] - 2026-07-29

### Added

- Added cached-history coverage to ticker headers and Statistics, with
  still-selectable muted range controls when a fixed range exceeds the observed
  span.
- Added exact price and date/time labels beside the chart cursor when the
  viewport has room.
- Added an optional provider capability for forward and reverse stock splits,
  implemented by the Alpaca adapter with bounded batching and pagination.

### Changed

- Map price traces and volume columns to actual timestamps across the selected
  range, leaving long no-observation intervals blank instead of stretching
  sparse data into a continuous plateau. Left/Right navigation skips those
  empty timestamp columns.
- Prefer a valid provider snapshot market cap; otherwise reconcile dated SEC
  share estimates through intervening split ratios before multiplying by the
  current price. A required split-coverage failure leaves the local estimate
  unavailable.
- Cache successful and empty per-symbol split coverage for up to 24 hours so
  five-minute price refreshes do not repeat the same corporate-action queries.

### Fixed

- Keep manually dispatched release workflows build-only even when they are
  launched from a tag; only a `v*` tag push may publish GitHub release assets.
- Stop terminal input reporting while still in raw mode, retire the input
  reader and cache workers, drain pending events through a bounded quiet
  period, and flush unread terminal events before restoring the shell. This
  prevents delayed SGR mouse coordinates from appearing at or being executed
  by the next prompt.
- Retain flat zero-volume/no-trade provider bars in the raw cache while
  excluding them from price endpoints, freshness, history coverage, timeframe
  selection, and detail charts.
- Keep chart price-axis padding at or above zero for low-priced securities.
- Make Overview sector and benchmark focus mutually exclusive for both mouse
  and keyboard navigation.

## [0.2.1] - 2026-07-29

### Changed

- Made Volume ordering, tile values, and sector-relative brightness range-aware:
  `1D` uses the selected snapshot's latest-session cumulative volume when
  available, with a cached bar-sum fallback, while longer ranges sum cached
  OHLCV volume inside the selected cutoff instead of reusing a daily snapshot.

### Documentation

- Refreshed the README's deterministic demo captures for the v0.2.0 Overview,
  Sector, and Ticker Detail interfaces.
- Clarified overlay-owned controls, synchronization progress, stale-label
  styling, cached-company search, and the private development provider boundary.
- Made generic cache and catalog lifecycle descriptions provider-neutral and
  corrected the documented `stock-api` news configuration names.

## [0.2.0] - 2026-07-28

### Added

- Added ticker-cell metric cycling in Sector and Starred views for price,
  relative and absolute gain, sector-relative gain, market cap, and volume.
- Added reversible ticker ordering and a center-out clockwise spiral
  presentation across Overview, Sector, and Starred heatmaps.
- Added `=`/`+` and `-` shortcuts to step toward shorter and longer chart
  ranges.
- Added first-run Alpaca onboarding with a highlighted OSC 8 signup link,
  explicit open, copy, skip, and demo choices, OSC 52 clipboard fallback,
  hidden key/secret entry, credential validation, and comment-preserving
  storage under `[providers.alpaca]` in the platform `config.toml`.
- Added SEC Financial Statement Data Set share extraction with explicit
  source, as-of date, method, and confidence metadata. The hierarchy supports
  issuer totals, reviewed equal-economic and converted share classes, and a
  low-confidence basic weighted-average fallback while excluding preferred
  and diluted securities.
- Added provider-neutral asset, market-data, and optional-news capabilities,
  with adapter selection through CLI, environment, or namespaced TOML.
- Added the optionally bearer-authenticated `stock-api` HTTP adapter, including
  a versioned interoperability contract, bounded validation/retries, and
  namespaced TOML configuration for compatible operator-supplied services.
- Added deterministic compact SEC catalog packaging and a daily GitHub Actions
  publisher for versioned Cloudflare R2 objects and manifests.
- Added Developer ID signing, hardened runtime, secure timestamps, Apple
  notarization, online ticket verification, and macOS release archives whose
  extracted executable is byte-checked against the accepted binary.

### Changed

- Sector cells now show one context-sensitive value below a vertically centered
  ticker, and starred cells use a thin frame when space permits.
- In color themes, Volume ordering now distinguishes sectors by hue and shows
  sector-relative volume through brightness; monochrome uses brightness only,
  and every other ordering retains the return scale.
- Normal status chrome now includes numeric synchronization progress, overlays
  use left-aligned columns, and the action rail identifies the app version.
- Alpaca credentials may live beside provider settings in `config.toml`;
  environment values remain higher precedence and older `credentials.env`
  files remain a read-only upgrade fallback.
- Sort direction now reverses loaded heatmap groups in memory, avoiding a
  synchronous full-market cache query.
- Replaced `p`/`n` sibling navigation with `Backspace`/`Space`; `Esc` is now
  the sole key for going up one route level.
- Isolated the default demo and live SQLite caches as `demo.sqlite3` and
  `market.sqlite3`.
- Made demo mode explicit with `--demo` or the onboarding choice; a normal
  launch without valid credentials no longer silently shows simulations.
- Added visible progress before credential checks and pre-TUI cache startup.
- Rank sector membership and market-cap ordering by the best available numeric
  size: estimated market cap when available, otherwise SEC-reported public
  float. Public float remains a labeled ranking proxy and is never displayed
  as market cap.
- Upgraded the SQLite cache to schema version 2 to retain ranking-proxy and
  share-estimate provenance across restarts and dated memberships.
- Resolve the first screen from local catalog data, then refresh and validate
  the compact R2 catalog in the background before reconciling provider assets.
- Build every release platform against one validated R2 catalog download while
  retaining the repository snapshot as the source-build fallback.
- Prefer a concise canonical common-stock symbol when the SEC lists a safe
  sibling choice, including `GOOG` for Alphabet, while preserving explicit
  classes whose per-share economics differ.

### Fixed

- Restrict GitHub release publication to exactly five platform archives so the
  build-only catalog artifact cannot become a separate public release asset.
- Redact echoed `stock-api` bearer tokens and terminal control characters from
  that adapter's errors before they reach status output or logs.
- Limit stale-data underlining to the ticker label instead of every value in
  the tile or benchmark footer item.
- Derive relative and absolute gains from the same period baseline and current
  displayed price, including intraday snapshot updates.
- Keep the detail header, chart endpoints, volume source, and stale-data
  timestamp on the same newer valid snapshot-or-bar observation.
- Compare starred tickers against their actual sector return and retain
  two-line metrics for a full 100-ticker sector at 80x24.
- Clean up simulated observations on a legacy demo-to-live transition so demo
  and Alpaca bars cannot interleave into corrupted charts.
- Load `STOCK_TUI_DB_PATH` from `.env` after dotenv initialization.
- Prevent large proxy-only issuers such as Alphabet from being displaced by
  every company with any calculated market cap.
- Invalidate a cached market cap when a catalog update changes the underlying
  share estimate or its provenance.
- Search SEC Frames independently of the latest bulk-file quarter while
  rejecting future-dated observations.
- Fail closed on unreviewed multi-class filings and gross public-float scale
  errors instead of retaining an older policy or silently correcting the SEC
  value.

### Documentation

- Documented the complete free Alpaca Paper Trading key setup, including
  one-time secret handling and dotenv lookup for installed binaries.
- Confirmed bring-your-own-key as the project architecture under ordinary
  Alpaca terms and added a public-display licensing inquiry checklist and
  ready-to-send request template.
- Documented Apple release credentials, secret-safe GitHub setup, and
  independent signature, notarization-ticket, and Gatekeeper verification.

## [0.1.1] - 2026-07-24

### Changed

- Updated the compatible Rust dependency lockfile, including correctness fixes
  in `futures-util`, Tokio, and TOML.

### Fixed

- Render the chart cursor with one centered middle dot per terminal cell instead
  of alternating between left- and right-aligned Braille subcolumns.

### Documentation

- Added current deterministic-demo captures of the market overview, Technology
  sector, and ticker detail screens to the README.

## [0.1.0] - 2026-07-23

### Added

- Initial Rust terminal application using Ratatui, Crossterm, Tokio, and
  bundled SQLite.
- StockTouch-inspired 3x3 overview with nine legacy economic sectors and up to
  100 color-coded companies per sector.
- Selectable S&P 500, Dow, and Nasdaq-100 overview status through explicitly
  labeled `SPY`, `DIA`, and `QQQ` ETF proxies.
- Ten ranges (`1D`, `1W`, `1M`, `3M`, `6M`, `1Y`, `2Y`, `5Y`, `10Y`, `ALL`)
  and market-cap, gain, volume, and alphabetical ordering.
- SGR-only mouse hover/click/drag/wheel input, keyboard navigation,
  paste-aware search, and terminal restoration on exit or panic.
- Context-sensitive `p`/`n` navigation across sectors, ordered ticker details,
  starred tickers, and benchmark charts, with current-order rank display.
- Responsive compact/full layouts, centered equal-cell heatmaps, half-block
  overview compression, true-color heat scales with contrast-aware focus,
  monochrome `NO_COLOR` mode, and a thin Braille price trace with softened area
  fill, fine-grained volume, and labeled axes.
- Ticker detail with price, return, OHLC, volume, market cap, sector context,
  company description, related news, persistent selection, and browser opening
  with OSC 52 clipboard fallback.
- Persistent favorites, dedicated Starred view, and local ticker/company-name
  search.
- Deterministic offline demo market values for 900 real SEC-catalog identities
  plus three benchmark ETF identities, all chart ranges, persistent simulation
  labeling, and clearly labeled simulated news.
- Alpaca adapter for active US equity assets, batched snapshots, paginated
  adjusted bars, and historical news, with secret redaction, request limiting,
  bounded retry/backoff, and feed fallback.
- Versioned issuer-universe support, dated sector memberships, and explicit
  mapping into the nine-sector legacy taxonomy.
- SQLite schema for companies, memberships, bars, snapshots, news,
  news-symbol relationships, favorites, and sync checkpoints, using WAL and
  transactional upserts.
- Background snapshot refresh, resumable two-year daily and all-available
  weekly history caching with a seven-day overlap, and lazy range-specific
  ticker/news synchronization.
- CLI modes for demo, offline cache, database/feed/refresh overrides, demo
  reset, and redacted effective-configuration output.
- Public architecture, provider/licensing, cache/sync, configuration,
  contribution, security, conduct, and financial-disclaimer documentation.

### Changed

- Aligned the three overview benchmark cells with the sector columns and
  constrained them to the market content pane.
- Moved price labels into the plot, replaced full-width Braille guides with
  terminal-stable middle dots, and added a high-contrast cursor intersection.
- Kept solid volume cells seam-free while restoring uniform-color fractional
  block caps for eighth-cell height precision.
- Replaced repeated full-history timeframe discovery during range changes with
  indexed per-symbol availability probes.
- Kept the Starred grid, detail rank, and adjacent-ticker navigation on the
  same globally sorted favorites list.

[Unreleased]: https://github.com/chatcode-lab/stock-tui/compare/v0.2.7...HEAD
[0.2.7]: https://github.com/chatcode-lab/stock-tui/compare/v0.2.6...v0.2.7
[0.2.6]: https://github.com/chatcode-lab/stock-tui/compare/v0.2.5...v0.2.6
[0.2.5]: https://github.com/chatcode-lab/stock-tui/compare/v0.2.4...v0.2.5
[0.2.4]: https://github.com/chatcode-lab/stock-tui/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/chatcode-lab/stock-tui/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/chatcode-lab/stock-tui/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/chatcode-lab/stock-tui/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/chatcode-lab/stock-tui/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/chatcode-lab/stock-tui/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/chatcode-lab/stock-tui/releases/tag/v0.1.0
