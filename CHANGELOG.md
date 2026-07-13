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
- Seven ranges (`1D`, `1W`, `1M`, `3M`, `6M`, `1Y`, `5Y`) and market-cap,
  gain, volume, and alphabetical ordering.
- Mouse hover/click/wheel input, keyboard navigation, paste-aware search, and
  terminal restoration on exit or panic.
- Responsive compact/full layouts, half-block overview compression, true-color
  heat scales, monochrome `NO_COLOR` mode, and Braille price charts with volume
  sparklines.
- Ticker detail with price, return, OHLC, volume, market cap, sector context,
  company description, related news, and browser opening.
- Persistent favorites, dedicated Starred view, and local ticker/company-name
  search.
- Deterministic offline demo data for 900 companies, all chart ranges, and
  clearly labeled simulated news.
- Alpaca adapter for active US equity assets, batched snapshots, paginated
  adjusted bars, and historical news, with secret redaction, request limiting,
  bounded retry/backoff, and feed fallback.
- Versioned issuer-universe support, dated sector memberships, and explicit
  mapping into the nine-sector legacy taxonomy.
- SQLite schema for companies, memberships, bars, snapshots, news,
  news-symbol relationships, favorites, and sync checkpoints, using WAL and
  transactional upserts.
- Background snapshot refresh, resumable five-year daily history caching with
  a seven-day overlap, and lazy range-specific ticker/news synchronization.
- CLI modes for demo, offline cache, database/feed/refresh overrides, demo
  reset, and redacted effective-configuration output.
- Public architecture, provider/licensing, cache/sync, configuration,
  contribution, security, conduct, and financial-disclaimer documentation.

[Unreleased]: https://github.com/chatcode-lab/stock-tui/commits/main
