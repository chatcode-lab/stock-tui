# Changelog

All notable changes to `stock-tui` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html) after its initial
pre-1.0 releases.

## [Unreleased]

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
- Responsive compact/full layouts, centered equal-cell heatmaps, half-block
  overview compression, true-color heat scales with contrast-aware focus,
  monochrome `NO_COLOR` mode, and a thin Braille price trace with softened area
  fill, seamless cell-filled volume, and labeled axes.
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
- Made the chart cursor and volume histogram independent of terminal font
  glyph alignment by painting their geometry with cell backgrounds.
- Replaced repeated full-history timeframe discovery during range changes with
  indexed per-symbol availability probes.

[Unreleased]: https://github.com/chatcode-lab/stock-tui/commits/main
