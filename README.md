# stock-tui

`stock-tui` is a mouse-first terminal stock heatmap inspired by the visual
model of StockTouch. It turns a broad US equity universe into a dense 3x3
market map, lets you open a sector's top 100 companies, and then drills into a
ticker with price, volume, statistics, and related news.

This is an independent open-source project. It is not affiliated with,
endorsed by, or a continuation of StockTouch or its creators.

The client is written in Rust with Ratatui, Crossterm, Tokio, and SQLite. It is
read-only: it displays market information and does not place orders.

The iterative Codex work performed through chatcode.dev is documented in the
[prompt-to-commit build log](PROMPTS.md).

> **Project status:** early, pre-1.0 software. The cache format and provider
> behavior may change between minor releases. All market information may be
> delayed, incomplete, or wrong. `stock-tui` is not investment advice.

## Screenshots

These captures use the deterministic offline demo. Company identities are real
SEC-catalog entries; all displayed prices, returns, volumes, rankings, and news
are simulated.

### Market Overview

[![Nine-sector stock market heatmap in stock-tui](docs/screenshots/market-overview.png)](docs/screenshots/market-overview.png)

### Sector View

[![Technology sector top-100 heatmap in stock-tui](docs/screenshots/technology-sector.png)](docs/screenshots/technology-sector.png)

### Ticker Detail

[![NVDA detail view with price and volume charts in stock-tui](docs/screenshots/ticker-detail.png)](docs/screenshots/ticker-detail.png)

## What It Does

- Displays nine economic sectors in a 3x3 overview, with up to 100 companies
  per sector.
- Shows S&P 500, Dow, and Nasdaq-100 performance through the liquid `SPY`,
  `DIA`, and `QQQ` ETF proxies in the overview footer.
- Colors each ticker from bright red through neutral gray to bright green from
  its return over `1D`, `1W`, `1M`, `3M`, `6M`, `1Y`, `2Y`, `5Y`, `10Y`, or
  all available history.
- Reorders tickers by estimated market cap with SEC public-float fallback,
  gain, volume, or symbol.
- Provides responsive sector grids and a ticker detail screen with a
  Braille-resolution price trace, softly filled tint, price/time axes,
  fine-grained volume histogram, current-order rank, statistics, company
  context, and news.
- Supports mouse hover, clicking, wheel input, keyboard navigation, and
  terminal resize events.
- Searches the local issuer catalog by symbol or company name.
- Persists starred tickers and emphasizes them in every heatmap.
- Opens immediately from a local SQLite cache while network synchronization
  proceeds in the background.
- Refreshes its compact SEC-derived issuer catalog from a public R2 object,
  while retaining a validated catalog inside every release for offline and
  outage-safe startup.
- Runs an explicitly selected demo, via `--demo` or onboarding, using 900 real
  SEC-catalog issuer identities plus three benchmark ETF identities, with
  deterministic, clearly labeled simulated market values.

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

Start the live client:

```bash
cargo run --release
```

Use `stock-tui --help` or `stock-tui -h` to list all command-line options.

When no credentials are configured, `stock-tui` shows Alpaca's registration
URL as a highlighted OSC 8 terminal link and waits: press `Enter` to open it,
`c` to copy it through the terminal clipboard, `d` to start demo mode, or `Esc`
to continue directly to credential entry. It accepts the key ID and secret
through hidden terminal input, validates the pair, and stores it in
`credentials.env` below the platform configuration directory. Existing
environment or working-directory `.env` values skip onboarding.

Use the simulated market explicitly when testing without provider data:

```bash
cargo run --release -- --demo
```

The first demo run selects 100 real SEC-catalog identities per sector and
generates simulated prices, rankings, multiple chart timeframes, volume, and
news, then stores them in SQLite. The persistent `SIMULATED` badge distinguishes
the demo from live Alpaca data. Demo and live mode use separate default
databases, `demo.sqlite3` and `market.sqlite3`, so adding credentials after a
demo run cannot combine their observations. Later runs reuse the versioned demo
database. Regenerate it with:

```bash
cargo run --release -- --demo --reset-demo
```

`--reset-demo` clears **the entire selected database**, including favorites and
any live cache, before regenerating it. This normally affects only the default
demo database; use a dedicated `--db` path when overriding it. No Alpaca account
or network connection is used in demo mode. Current versioned demo caches are
reused, and the old fabricated-identity demo is upgraded automatically.

### Use Alpaca Data

Alpaca's free Paper Trading account can issue credentials for the Basic market
data plan; no funded brokerage account is required. To configure it:

1. Create or sign in to an
   [Alpaca Trading API account](https://app.alpaca.markets/account/login).
2. Select **Paper Trading** in the dashboard account switcher.
3. Open the dashboard API Keys panel and generate a key pair. Store the secret
   immediately: Alpaca displays it only once, and regenerating a pair
   invalidates the previous credentials.
4. Launch `stock-tui` and enter both values when prompted. Input is not echoed,
   and the pair is validated before it is stored. Alternatively, put both
   values in the process environment or in a private `.env` file in the
   directory from which `stock-tui` is launched:

```dotenv
ALPACA_API_KEY=your-own-key-id
ALPACA_API_SECRET=your-own-secret
```

For a source checkout, `.env.example` remains a ready-to-fill override:

```bash
cp .env.example .env
chmod 600 .env
```

Never commit `.env`, credentials, a populated database, or diagnostic output
that might contain account information. Do not paste secrets into commands that
will remain in shell history. Start the client with:

```bash
cargo run --release
```

For a prebuilt binary, run `stock-tui` from the private directory containing
`.env`, or export the variables in the launching shell. The dotenv loader
searches the current directory and its parents; it does not automatically
search beside an installed executable. Without either override, onboarding
uses `<config_dir>/credentials.env`; find the exact directory with
`stock-tui --print-config`. The app reads asset metadata from Alpaca's paper
endpoint but never submits orders. If your credentials belong to another
Alpaca environment, set `STOCK_TUI_TRADING_URL` to its matching HTTPS base URL.

The default `iex` feed works with Alpaca's Basic plan. See
[Data Providers](docs/data-providers.md) before selecting `sip` or
`delayed_sip`; access is controlled by the user's Alpaca subscription.
Alpaca's current
[Paper Trading guide](https://alpaca.markets/learn/start-paper-trading)
contains the authoritative dashboard steps if its interface changes.

Alpaca is the first provider adapter and remains the default. The provider is
selected explicitly with `--provider alpaca`, `STOCK_TUI_PROVIDER=alpaca`, or
`provider = "alpaca"` in `config.toml`. A provider-neutral HTTP adapter is also
available for compatible separately operated services:

```bash
stock-tui --provider stock-api --stock-api-url http://127.0.0.1:8787
```

Compatible services may be unauthenticated, or may require the optional
environment-only `STOCK_TUI_STOCK_API_TOKEN` bearer token. There is no CLI or
TOML token setting, and `--print-config` omits both its value and presence.
For the private development endpoint:

```bash
read -rsp "Stock API token: " STOCK_TUI_STOCK_API_TOKEN
printf '\n'
export STOCK_TUI_STOCK_API_TOKEN
stock-tui --provider stock-api --stock-api-url https://stock.chatcode.dev/api
unset STOCK_TUI_STOCK_API_TOKEN
```

The project-operated endpoint is for explicitly authorized development tests,
not a licensed public market-data service. See the
[Stock API HTTP Contract](docs/stock-api-contract.md) before operating a
service. The synchronization layer keeps separate asset, market-data, and
optional-news capabilities so provider payloads and authentication do not leak
into storage and UI code.

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

Download the archive for your operating system and CPU from
[GitHub Releases](https://github.com/chatcode-lab/stock-tui/releases), verify
the archive against the attached `SHA256SUMS`, extract it, and place `stock-tui`
(or `stock-tui.exe`) on `PATH`.

| Platform | Release asset |
| --- | --- |
| Linux x86_64 | `stock-tui-v<VERSION>-x86_64-unknown-linux-musl.tar.gz` |
| Linux ARM64 | `stock-tui-v<VERSION>-aarch64-unknown-linux-musl.tar.gz` |
| macOS Apple Silicon | `stock-tui-v<VERSION>-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `stock-tui-v<VERSION>-x86_64-apple-darwin.tar.gz` |
| Windows x86_64 | `stock-tui-v<VERSION>-x86_64-pc-windows-msvc.zip` |

The GitHub CLI can display and download the latest available assets:

```bash
gh release view --repo chatcode-lab/stock-tui
gh release download --repo chatcode-lab/stock-tui
```

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
| `Esc` | Close an overlay or go up one level |
| `Backspace` | Open the previous sector or ticker, wrapping at either end; delete text in Search |
| `Space` | Open the next sector or ticker, wrapping at either end; insert a space in Search |
| `/` | Search cached companies by ticker or name |
| `s` | Open ticker ordering |
| `F` | Open starred tickers |
| `f` | Star or unstar the focused ticker |
| `[` / `]` | Previous / next date range |
| `1` through `9` | Select `1D` through `10Y` directly |
| `0` | Select all available history |
| `g`, then `c s h e t f i m u` | Open Consumer, Services, Healthcare, Energy, Technology, Financial, Industrial, Materials, or Utilities |
| `Alt`/`Meta` + sector letter | Optional direct form of the same sector shortcut |
| `Tab` | Cycle Chart, Statistics, and News in compact ticker view |
| `r` | Request an immediate broad-market snapshot refresh |
| `S` | Open read-only data status |
| `?` | Open keyboard help |
| `q` or `Ctrl-C` | Quit and restore the terminal |

On ticker detail, Left/Right (or `h`/`l`) moves the chart cursor while
Up/Down (or `k`/`j`) selects the related-news row; `Enter` opens it.
`Backspace` and `Space` open the previous or next ticker in the originating
sector, starred list, or benchmark order while preserving the selected range
and ordering.

The `g` sector prefix applies only to the immediately following key. While it
is armed, `Esc` or `Backspace` cancels it without navigating; any other
non-sector key cancels the prefix and keeps its normal action. Mouse input and
opening an overlay also cancel a pending prefix.

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
OSC 8 links, OSC 52 clipboard access, and RGB color. `NO_COLOR=1 stock-tui`
selects the monochrome palette when color is not usable.

## Data And Cache

The live client combines a versioned SEC-derived candidate catalog with
provider snapshots, adjusted bars, asset names/exchanges, and ticker news. It
stores normalized records in a per-user SQLite database in WAL mode. At most
once every 12 hours by default, a background task checks the compact catalog at
`https://stock.chatcode.dev/catalog/sec-catalog.json`. It checks at startup and
keeps that cadence while the app remains open. The first screen does not wait
for the request. A valid download is cached below the platform cache directory,
applied to SQLite, and followed by provider universe reconciliation; an
invalid, older, unavailable, or oversized response cannot replace the last
valid cache. Offline mode uses that cache when it is at least as new as the
embedded release catalog, otherwise it uses the embedded catalog.

The catalog includes SEC public float as a ranking proxy plus
provenance-tagged common-share estimates where the official filings support
one. On startup the client refreshes candidate snapshots, estimates ordinary
market cap from shares and price, and selects each sector's top 100 by
estimated cap or numeric public float when cap is unavailable. Public float is
never displayed as market cap.
It then resumes two years of daily bars plus all provider-available weekly
history for the selected 900 companies and the three benchmark ETF proxies.
Both history plans use a seven-day overlap after their initial backfill. It
lazily requests range-appropriate bars and 20 newest headlines when a ticker is
opened.
In live mode, the broad-market snapshot refresh runs immediately on startup
and every five minutes by default; `r` starts one immediately and restarts that
timer. Opening a ticker or changing its range separately triggers a lazy detail
request. Demo and offline modes never schedule remote refreshes. `S` only opens
the status panel; it does not start synchronization.

The current adapter is limited to Alpaca US equities. Feed selection changes
US venue coverage; it does not enable non-US markets. Additional countries,
currencies, sessions, and licensed providers need explicit future adapters.
The public catalog endpoint contains only the compact SEC-derived issuer
catalog; it is not a market-price or news proxy.

Alpaca's Basic plan currently provides a 200 historical-request-per-minute
limit and real-time US equity coverage from IEX, a single exchange. The default
client limiter is 180 requests per minute. IEX prices and volumes are not the
same as consolidated whole-market SIP figures. Provider limits and terms can
change; consult Alpaca's current documentation.

The local cache is for the credential holder's use. **Alpaca states that its
API data cannot be redistributed under ordinary access terms.** Do not publish
or ship a populated Alpaca cache. The project intentionally uses a
bring-your-own-key model and has no shared-key market-data proxy or public
price/news backend.
See [Requesting Public-Display Permission](docs/data-providers.md#requesting-public-display-permission)
before proposing any no-key service.

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

- [Prompt-to-Commit Build Log](PROMPTS.md)
- [Architecture](docs/architecture.md)
- [Data Providers and Licensing](docs/data-providers.md)
- [Cache and Synchronization](docs/cache-and-sync.md)
- [Configuration](docs/configuration.md)
- [SEC Catalog R2 Operations](infra/cloudflare/README.md)
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
