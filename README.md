# stock-tui

`stock-tui` is a mouse-first terminal stock heatmap inspired by the visual
model of StockTouch. It turns a broad US equity universe into a dense 3x3
market map, lets you open a sector's top 100 companies, and then drills into a
ticker with price, volume, statistics, and related news.

This is an independent open-source project. It is not affiliated with,
endorsed by, or a continuation of StockTouch or its creators.

The client is written in Rust with Ratatui, Crossterm, Tokio, and SQLite. It is
read-only: it displays market information and does not place orders.

> **Project status:** early, pre-1.0 software. The cache format and provider
> behavior may change between minor releases. All market information may be
> delayed, incomplete, or wrong. `stock-tui` is not investment advice.

## What It Does

- Displays nine economic sectors in a 3x3 overview, with up to 100 companies
  per sector.
- Shows S&P 500, Dow, and Nasdaq-100 performance through the liquid `SPY`,
  `DIA`, and `QQQ` ETF proxies in the overview footer.
- Colors each ticker from bright red through neutral gray to bright green from
  its return over `1D`, `1W`, `1M`, `3M`, `6M`, `1Y`, `2Y`, `5Y`, `10Y`, or
  all available history.
- Reorders tickers by market capitalization, gain, volume, or symbol.
- Provides responsive sector grids and a ticker detail screen with a
  Braille-resolution price trace, softly filled tint, price/time axes,
  gap-free cell-filled volume, statistics, company context, and news.
- Supports mouse hover, clicking, wheel input, keyboard navigation, and
  terminal resize events.
- Searches the local issuer catalog by symbol or company name.
- Persists starred tickers and emphasizes them in every heatmap.
- Opens immediately from a local SQLite cache while network synchronization
  proceeds in the background.
- Runs without credentials using 900 real SEC-catalog issuer identities plus
  three benchmark ETF identities, with deterministic, clearly labeled
  simulated market values.

The heat scale is symmetric around zero and capped using the visible market's
90th-percentile absolute move. This keeps one extreme ticker from flattening
the rest of the palette. Missing or zero-return data appears neutral; data more
than 72 hours old keeps a contrast-aware foreground and is underlined as a
freshness hint.

## Why Rust

Go would also be a reasonable implementation language, but Rust is a stronger
fit for this client: Ratatui provides precise cell and canvas rendering
support, Crossterm supplies portable mouse and keyboard events, Tokio handles
background provider work, and `rusqlite` can bundle SQLite into one native
binary. The result has no language runtime to install and keeps redraw and
cache paths explicit.

## Quick Start

### Requirements

- Rust 1.95 or newer for a source build
- A native C compiler and linker for bundled SQLite and TLS dependencies
- A modern terminal with UTF-8 and mouse reporting
- At least 60 columns by 20 rows; 120 by 36 or larger enables the full layout
- True-color support for the intended palette (256-color terminals still run,
  but color reproduction depends on the terminal)

With both Alpaca variables present in `.env` or the process environment, start
the live client:

```bash
cargo run --release
```

Use the offline simulated market only when testing without provider data:

```bash
cargo run --release -- --demo
```

The first demo run selects 100 real SEC-catalog identities per sector and
generates simulated prices, rankings, multiple chart timeframes, volume, and
news, then stores them in SQLite. The persistent `SIMULATED` badge distinguishes
the demo from live Alpaca data. Later runs reuse the versioned demo database.
Regenerate it with:

```bash
cargo run --release -- --demo --reset-demo
```

`--reset-demo` clears **the entire selected database**, including favorites and
any live cache, before regenerating it. Use a dedicated `--db` path when that
data matters. No Alpaca account or network connection is used in demo mode. If
neither Alpaca credential variable is set, demo mode is selected automatically
unless `--offline` was explicitly requested. Current versioned demo caches are
reused, and the old fabricated-identity demo is upgraded automatically. Use a
dedicated database when switching between demo and live modes.

### Use Alpaca Data

Create an Alpaca account, issue API credentials in the Alpaca dashboard, and
put both variables in your shell environment or a local `.env` file:

```dotenv
ALPACA_API_KEY=your-own-key-id
ALPACA_API_SECRET=your-own-secret
```

Never commit `.env`, credentials, a populated database, or diagnostic output
that might contain account information. Start the client with:

```bash
cargo run --release
```

The default asset-metadata endpoint is Alpaca's paper environment, which works
with free paper account credentials and does not require a funded live
brokerage account. The app reads asset metadata from that endpoint but never
submits orders. If your credentials belong to another Alpaca environment, set
`STOCK_TUI_TRADING_URL` to its matching HTTPS base URL.

The default `iex` feed works with Alpaca's Basic plan. See
[Data Providers](docs/data-providers.md) before selecting `sip` or
`delayed_sip`; access is controlled by the user's Alpaca subscription.

To inspect the effective non-secret settings and resolved paths:

```bash
cargo run --release -- --print-config
```

Credentials are redacted from this output. To prohibit all network access and
use an existing live-data cache:

```bash
cargo run --release -- --offline
```

`--offline` does not manufacture missing data. Run online at least once to
populate a live cache, or use `--demo` for a self-contained experience.

## Install

### Prebuilt Binaries

When a release has attached artifacts, download the archive for your operating
system and CPU from
[GitHub Releases](https://github.com/chatcode-lab/stock-tui/releases), verify
the archive against the attached `SHA256SUMS`, extract it, and place `stock-tui`
(or `stock-tui.exe`) on `PATH`.

The GitHub CLI can display and download the latest available assets:

```bash
gh release view --repo chatcode-lab/stock-tui
gh release download --repo chatcode-lab/stock-tui
```

Source builds remain the canonical installation path until a release lists a
binary for your platform.

### Build From Source

```bash
git clone https://github.com/chatcode-lab/stock-tui.git
cd stock-tui
cargo build --release --locked
./target/release/stock-tui --demo
```

Install the current checkout into Cargo's binary directory:

```bash
cargo install --path . --locked
stock-tui --demo
```

## Controls

Every visible rail control, sector, ticker, range, sort option, detail tab, and
news row can be clicked with the left mouse button.

| Input | Action |
| --- | --- |
| Mouse move | Select a sector, benchmark, ticker, or news row; move the chart cursor |
| Left click | Activate the control, sector, ticker, tab, or news item |
| Wheel on overview/sector | Move to the previous or next date range |
| Wheel on ticker chart | Move the selected chart sample |
| Arrow keys or `h` `j` `k` `l` | Move sector, ticker, sort, chart, or news selection |
| `Enter` | Open the selected sector, ticker, news item, or overlay choice |
| `Esc` or `Backspace` | Close an overlay or go back |
| `/` | Search cached companies by ticker or name |
| `s` | Open ticker ordering |
| `F` | Open starred tickers |
| `f` | Star or unstar the focused ticker |
| `[` / `]` | Previous / next date range |
| `1` through `9` | Select `1D` through `10Y` directly |
| `0` | Select all available history |
| `Alt`/`Meta` + `c s h e t f i m u` | Open Consumer, Services, Healthcare, Energy, Technology, Financial, Industrial, Materials, or Utilities |
| `Tab` | Cycle Chart, Statistics, and News in compact ticker view |
| `r` | Request an immediate broad-market snapshot refresh |
| `S` | Open read-only data status |
| `?` | Open keyboard help |
| `q` or `Ctrl-C` | Quit and restore the terminal |

On ticker detail, Left/Right (or `h`/`l`) moves the chart cursor while
Up/Down (or `k`/`j`) selects the related-news row; `Enter` opens it.

In search, type or paste a query, use Up/Down to select a result, `Enter` to
open it, `Ctrl-U` to clear the query, and `Esc` to close. Search is local and
returns at most 20 catalog matches. Activating a headline asks the operating
system to open its provider URL in the default browser. If no browser can be
launched, the URL is copied through the terminal's OSC 52 clipboard protocol
instead.

On ANSI terminals, `stock-tui` explicitly requests all-motion tracking with
SGR mouse encoding (`1003` + `1006`). Its click, hover, drag, and wheel reports
therefore travel as text input and do not depend on legacy X10/onBinary mouse
transport.

## Responsive Layout

- Below 60x20, the app shows a resize prompt instead of drawing overlapping
  content.
- From 60x20, compact mode keeps the action rail and adapts the number of
  sector columns to available width.
- At 120x36 and above, ticker detail becomes a split workspace with chart and
  description on the left and statistics and news on the right.
- The overview always preserves the 3x3 sector model. Short terminals compress
  each 10x10 sector into paired half-block rows so all 100 color signals remain
  visible.
- Sector panels and ticker tiles use one fixed cell size at a given viewport.
  Any indivisible rows or columns become balanced outer padding instead of
  stretching selected tiles.

Terminals differ in their handling of mouse motion, Braille/half-block glyphs,
OSC 52 clipboard access, and RGB color. `NO_COLOR=1 stock-tui` selects the
monochrome palette when color is not usable.

## Data And Cache

The live client combines a versioned SEC-derived candidate catalog with Alpaca
snapshots, adjusted bars, asset names/exchanges, and ticker news. It stores
normalized records in a per-user SQLite database in WAL mode. On startup it
refreshes candidate snapshots, updates top-100 sector membership where a
current market cap can be estimated, and resumes two years of daily bars plus
all provider-available weekly history for the selected 900 companies and the
three benchmark ETF proxies. Both history plans use a seven-day overlap after
their initial backfill. It lazily requests range-appropriate bars and 20 newest
headlines when a ticker is opened.
In live mode, the broad-market snapshot refresh runs immediately on startup
and every five minutes by default; `r` starts one immediately and restarts that
timer. Opening a ticker or changing its range separately triggers a lazy detail
request. Demo and offline modes never schedule remote refreshes. `S` only opens
the status panel; it does not start synchronization.

The current adapter is limited to Alpaca US equities. Feed selection changes US
venue coverage; it does not enable non-US markets. Additional countries,
currencies, sessions, and licensed providers need explicit future adapters.

Alpaca's Basic plan currently provides a 200 historical-request-per-minute
limit and real-time US equity coverage from IEX, a single exchange. The default
client limiter is 180 requests per minute. IEX prices and volumes are not the
same as consolidated whole-market SIP figures. Provider limits and terms can
change; consult Alpaca's current documentation.

The local cache is for the credential holder's use. **Alpaca states that its
API data cannot be redistributed under ordinary access terms.** Do not publish
or ship a populated Alpaca cache. A future no-key fallback backend is only
viable with market-data and news licenses that expressly permit redistribution;
it is not part of the current client.

See [Cache and Sync](docs/cache-and-sync.md) for the schema and lifecycle, and
[Configuration](docs/configuration.md) for every supported option.

## Sector Model

The project intentionally preserves StockTouch's nine-sector presentation:
Consumer, Services, Healthcare, Energy, Technology, Financial, Industrial,
Materials, and Utilities. This is a legacy visualization taxonomy, not the
current 11-sector GICS taxonomy. For example, Communication Services maps to
Services and Real Estate maps to Financial. Mapping and catalog caveats are
documented in [Data Providers](docs/data-providers.md).

## Documentation

- [Architecture](docs/architecture.md)
- [Data Providers and Licensing](docs/data-providers.md)
- [Cache and Synchronization](docs/cache-and-sync.md)
- [Configuration](docs/configuration.md)
- [Contributing](CONTRIBUTING.md)
- [Security Policy](SECURITY.md)
- [Changelog](CHANGELOG.md)
- [Code of Conduct](CODE_OF_CONDUCT.md)

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Provider tests use controlled local fixtures and must never use real secrets.
See [CONTRIBUTING.md](CONTRIBUTING.md) before proposing a new data source or
changing the sector taxonomy.

## Financial Disclaimer

`stock-tui` is an informational visualization project, not a broker, exchange,
investment adviser, research provider, or source of official quotations. It
does not account for every venue, corporate action, symbol change, data error,
or latency condition. Demo values are simulated. Historical performance does
not predict future results. Verify important information with an authorized
source and make financial decisions independently.

## License

The source code is available under the [MIT License](LICENSE). That license
applies to this project's code and documentation, not to third-party market
data, news, trademarks, or provider content.
