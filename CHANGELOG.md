# Changelog

All notable changes to `stock-tui` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html) after its initial
pre-1.0 releases.

## [Unreleased]

### Added

- Added first-run Alpaca onboarding with a highlighted OSC 8 signup link,
  explicit open, copy, skip, and demo choices, OSC 52 clipboard fallback,
  hidden key/secret entry, credential validation, and a managed
  `credentials.env` below the platform config directory.
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

[Unreleased]: https://github.com/chatcode-lab/stock-tui/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/chatcode-lab/stock-tui/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/chatcode-lab/stock-tui/releases/tag/v0.1.0
