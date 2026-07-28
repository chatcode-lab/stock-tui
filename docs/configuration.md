# Configuration

`stock-tui` has command-line flags, environment variables, a local dotenv
file, a small onboarding-managed credentials file, and a strict TOML file.

## Precedence

For settings that exist in more than one place, the effective order is:

1. Command-line flag
2. Process environment (including values loaded from `.env`)
3. `<config_dir>/config.toml`
4. Built-in default

An already exported process variable wins over the corresponding value in
`.env`. TOML rejects unknown keys to catch spelling mistakes. Use:

```bash
stock-tui --print-config
```

to show resolved paths and non-secret values. Credential values are redacted.

Credential lookup is narrower and applies only when `provider = "alpaca"`:

1. A complete `ALPACA_API_KEY` and `ALPACA_API_SECRET` pair from the process or
   working-directory `.env`
2. `<config_dir>/credentials.env`
3. Interactive onboarding for a normal online launch

Values from different sources are never combined. `--demo`, `--offline`,
`--print-config`, and the credential-free `stock-api` adapter do not launch
Alpaca onboarding or read its managed credential file.

## Credentials

Both variables must be set together and neither may be empty:

| Environment variable | Purpose |
| --- | --- |
| `ALPACA_API_KEY` | The local user's Alpaca API key ID. |
| `ALPACA_API_SECRET` | The matching Alpaca API secret. |

### Obtain A Free Personal Key

1. Create or sign in to an
   [Alpaca Trading API account](https://app.alpaca.markets/account/login).
2. Use the dashboard account switcher to select **Paper Trading**.
3. Open the API Keys panel and generate a key pair.
4. Record the secret when it appears. Alpaca shows it only once; regenerating
   the pair invalidates the old key and secret.
5. Launch `stock-tui` and enter both values through the hidden prompts, or set
   the values under the names used by `stock-tui`:

```dotenv
ALPACA_API_KEY=your-own-key-id
ALPACA_API_SECRET=your-own-secret
```

Alpaca's
[Paper Trading setup guide](https://alpaca.markets/learn/start-paper-trading)
documents the current dashboard flow. Its
[Market Data API plan page](https://docs.alpaca.markets/us/docs/about-market-data-api)
is authoritative for current free-plan coverage and limits.

The default Trading API endpoint is Alpaca's paper environment, so free paper
account credentials work without a funded live brokerage account. `stock-tui`
uses that endpoint only to read the active US-equity asset directory; it does
not submit or manage orders. Keys issued for a different Alpaca environment
must be paired with the corresponding `STOCK_TUI_TRADING_URL` override.

If a normal online launch has no complete pair, onboarding prints the signup
URL as a highlighted OSC 8 terminal link and waits for a single key. Redirected
output retains the plain URL without terminal escapes. `Enter` opens it in the
default browser, `c` sends it through OSC 52, `d` starts demo mode without
credentials, and `Esc` continues directly to credential entry. A failed browser
launch still falls back to OSC 52. Both credential input fields are hidden. The
app reports credential validation and cache preparation before slow work. It
validates the pair against Alpaca's Paper Trading account endpoint before
writing it to `<config_dir>/credentials.env`, then starts the normal market
view.

The onboarding demo choice applies only to the current launch. It uses the
isolated default `demo.sqlite3` cache unless `--db` or `STOCK_TUI_DB_PATH`
explicitly selected another path.

On macOS and Linux, the managed file is forced to owner read/write permissions
(`0600`). On Windows it is kept below the current user's platform configuration
directory. The file contains the two raw dotenv values; it is not encrypted.
Do not share, synchronize, or commit it. Credentials are never written to
SQLite, `config.toml`, logs, or terminal output.

Only an Alpaca `401` response proves that a configured pair is invalid.
Provider downtime, rate limits, malformed responses, and entitlement errors do
not erase it or force re-entry; the app continues with the local cache and the
normal synchronizer retries. A rejected environment pair can fall back to a
different validated managed pair, but the stale environment variables should
still be removed or updated.

Debug and release binaries use the same environment variable names and `.env`
format. The dotenv loader starts at the process working directory and searches
its parents; it does not search beside an installed executable automatically.
Keep that file private and outside version control. Do not put credentials in
`config.toml`, command history, screenshots, issues, or release assets.

For an interactive installation, export both variables in the launching shell,
start `stock-tui` from a dedicated private directory containing `.env`, or use
onboarding. For a service or container, inject credentials with the platform's
secret or environment facility instead of relying on an interactive prompt.
On macOS and Linux, restrict a manually created dotenv file to its owner with
`chmod 600 .env`. The repository's `.env.example` contains only empty
placeholders and is safe to commit; a filled `.env` is not.

## Command-Line Flags

| Flag | Meaning |
| --- | --- |
| `--demo` | Use the deterministic simulated market, even if credentials exist. |
| `--reset-demo` | Clear the entire selected database and rebuild demo records; requires `--demo`. |
| `--offline` | Never start remote synchronization; render the selected cache. |
| `--db <PATH>` | Override the SQLite database path. |
| `--provider <PROVIDER>` | Select `alpaca` or the provider-neutral `stock-api` HTTP adapter. |
| `--stock-api-url <URL>` | Set the `stock-api` base URL; it excludes the appended `/v1` route prefix. |
| `--stock-api-news <BOOL>` | Enable or omit the optional `stock-api` news capability. |
| `--feed <FEED>` | Select `iex`, `delayed_sip`, or `sip`. |
| `--catalog-url <URL>` | Override the compact SEC catalog endpoint. |
| `--catalog-refresh-hours <N>` | Recheck the catalog after `N` hours, clamped to 1 through 168. |
| `--refresh-seconds <N>` | Set snapshot refresh cadence, clamped to 30 through 86,400 seconds. |
| `--print-config` | Print redacted effective settings and exit. |
| `-h`, `--help` | Print CLI help. |
| `-V`, `--version` | Print the binary version. |

`--offline` always opens the selected cache without networking, including when
credentials are absent. Combine `--offline` with `--demo` only when the
selected database is intentionally a demo cache.

## Environment Variables

| Variable | Default | Notes |
| --- | --- | --- |
| `STOCK_TUI_DB_PATH` | Platform data dir plus `market.sqlite3`, or `demo.sqlite3` in demo mode | Equivalent to `--db`. |
| `STOCK_TUI_PROVIDER` | `alpaca` | Selects `alpaca` or `stock-api`. |
| `STOCK_TUI_STOCK_API_URL` | `https://stock.chatcode.dev/api` | Provider-neutral HTTP service base; HTTPS except for loopback development. |
| `STOCK_TUI_STOCK_API_NEWS` | `true` | Registers and requests the optional `/v1/news` capability. |
| `STOCK_TUI_FEED` | `iex` | `iex`, `delayed_sip`, or `sip`; entitlement remains provider-controlled. |
| `STOCK_TUI_REFRESH_SECONDS` | `300` | Equivalent to `--refresh-seconds`; clamped to 30..86,400. |
| `STOCK_TUI_CATALOG_URL` | `https://stock.chatcode.dev/catalog/sec-catalog.json` | Compact SEC-derived catalog; HTTPS is required except for loopback tests. |
| `STOCK_TUI_CATALOG_REFRESH_HOURS` | `12` | Maximum age before another catalog request; clamped to 1..168. |
| `STOCK_TUI_DATA_URL` | `https://data.alpaca.markets` | Alpaca Market Data base URL; mainly for controlled testing/proxies. |
| `STOCK_TUI_TRADING_URL` | `https://paper-api.alpaca.markets` | Alpaca paper Trading API base URL, used only for asset metadata. |
| `NO_COLOR` | Unset | Any value selects the monochrome heat palette. |
| `RUST_LOG` | `stock_tui=info,warn` | Tracing filter for daily files below `<cache_dir>/logs`. |

Changing Alpaca service URLs sends credentials to those hosts. Non-loopback
provider URLs must use HTTPS; plain HTTP is accepted only for local fixture
servers. Only point a live build at infrastructure you trust and control. URL
overrides do not waive provider terms or create redistribution rights.

The `stock-api` adapter never sends Alpaca credentials or any other application
authorization header. Its service base still needs to be trusted because it
controls the observations written to the local cache.

## TOML File

The file is `config.toml` in the platform configuration directory. Find the
exact `config_dir` with `--print-config`.

```toml
provider = "alpaca"
refresh_seconds = 300
catalog_url = "https://stock.chatcode.dev/catalog/sec-catalog.json"
catalog_refresh_hours = 12

[providers.alpaca]
feed = "iex"
request_limit_per_minute = 180
snapshot_batch_size = 100
history_batch_size = 50

# Advanced provider endpoints:
# data_url = "https://data.alpaca.markets"
# trading_url = "https://paper-api.alpaca.markets"

[providers.stock_api]
base_url = "https://stock.chatcode.dev/api"
news = true

# Local Worker:
# base_url = "http://127.0.0.1:8787"
```

Supported keys and validation:

| Key | Default | Accepted value |
| --- | --- | --- |
| `provider` | `alpaca` | A compiled provider adapter ID: `alpaca` or `stock-api` |
| `refresh_seconds` | `300` | Integer, clamped to 30..86,400 |
| `catalog_url` | Public `stock.chatcode.dev` catalog | HTTPS URL, or loopback HTTP for tests |
| `catalog_refresh_hours` | `12` | Integer, clamped to 1..168 |
| `providers.alpaca.feed` | `iex` | `iex`, `delayed_sip`, or `sip` |
| `providers.alpaca.request_limit_per_minute` | `180` | Integer, clamped to 1..200 |
| `providers.alpaca.snapshot_batch_size` | `100` | Integer, clamped to 1..500 |
| `providers.alpaca.history_batch_size` | `50` | Integer, clamped to 1..200 |
| `providers.alpaca.data_url` | Alpaca production data URL | HTTPS base URL, or loopback HTTP for tests |
| `providers.alpaca.trading_url` | Alpaca paper trading URL | HTTPS base URL, or loopback HTTP for tests |
| `providers.stock_api.base_url` | `https://stock.chatcode.dev/api` | HTTPS base URL without `/v1`, or loopback HTTP for local development |
| `providers.stock_api.news` | `true` | Boolean; omit the news capability and requests when false |

Credentials and the database path are intentionally absent from TOML. Use
onboarding or the environment for credentials, and `--db` /
`STOCK_TUI_DB_PATH` for the database path.

The flat Alpaca keys accepted by earlier releases remain compatible, but the
`[providers.alpaca]` namespace is preferred so future adapters can have
independent settings.

`https://stock.chatcode.dev/api` is reserved configuration, not a currently
deployed or licensed public market-data service. Use a compatible service that
you are authorized to operate, or point local Worker development at
`http://127.0.0.1:8787`. The complete versioned JSON contract is documented in
[Stock API HTTP Contract](stock-api-contract.md).

The runtime never polls the SEC. A background task rechecks the compact R2
catalog at startup and after each `catalog_refresh_hours` interval, validates
the complete catalog, and falls back to the newest valid cached or embedded
copy. A fresh cache suppresses the HTTP request. The first UI frame does not
wait for this work, and `--offline` disables it.
Catalog maintainers run `tools/build_sec_catalog.py` separately with
`--user-agent` or the `SEC_USER_AGENT` build-tool environment variable. See
[Data Providers](data-providers.md#catalog-build-process).

## Logs

Normal runs initialize daily, non-ANSI tracing files below
`<cache_dir>/logs`. Use `--print-config` to resolve `cache_dir`. `RUST_LOG`
accepts standard `tracing_subscriber` filter syntax, for example:

```bash
RUST_LOG=stock_tui=debug stock-tui --demo
```

Logs should not contain credential values, but may include provider errors and
operational context. Review and redact them before sharing. `--print-config`
exits before logging is initialized.

## Feed Selection

`iex` is the conservative default for Alpaca's individual Basic plan. IEX is
only one exchange and its price/volume observations differ from consolidated
SIP data.

`sip` asks for consolidated data and requires the appropriate subscription for
current snapshots. `delayed_sip` maps historical requests to SIP, ends those
requests 16 minutes before the current time, and allows the adapter's snapshot
fallback behavior. A configured label is not proof of entitlement; Alpaca can
return `403` or `422`, and the app reports the error or uses an allowed
fallback.

See [Data Providers](data-providers.md) for current official plan links and
redistribution restrictions.

Feed selection does not select a country or asset class. The current adapter
requests Alpaca `us_equity` assets only; eligible non-US data needs a future
provider implementation with explicit currency and session semantics.

`--feed` and `[providers.alpaca].feed` apply only to Alpaca. `stock-api`
normalizes its own licensed feed behind the documented HTTP contract, and the
TUI does not display an Alpaca feed label in that mode.

## Rate And Batch Tuning

The request limiter is a process-local token bucket. The default 180 requests
per minute leaves room below Alpaca's currently documented 200-per-minute Basic
historical limit. Lower it when other programs share the same account or when
provider responses indicate pressure.

Larger symbol batches reduce request count but increase payload size, response
latency, and the amount retried after a failure. Defaults are designed for the
broader candidate snapshot pool and the selected 900-company history universe
plus three benchmark ETF proxies. Increasing them does not increase account
entitlement and may exceed endpoint-specific symbol or response limits.

Transient requests use a 20-second timeout, up to three retries, exponential
delays starting at 250 milliseconds, and a 30-second cap. A provider
`Retry-After` header takes precedence within that cap.

## Database Profiles

Use explicit paths to keep independent caches:

```bash
stock-tui --demo --db "$HOME/.local/share/stock-tui/demo.sqlite3"
stock-tui --db "$HOME/.local/share/stock-tui/alpaca-iex.sqlite3" --feed iex
stock-tui --provider stock-api --stock-api-url http://127.0.0.1:8787 \
  --db "$HOME/.local/share/stock-tui/stock-api.sqlite3"
```

Paths shown are Linux examples. Quote paths containing spaces. The parent
directory is created automatically.

Do not point two configurations with different data licenses at the same
database unless their combined retention and use are permitted. Never place a
live database in a repository, web-synchronized public folder, or release.

## Examples

Demo with a fresh generated market:

```bash
stock-tui --demo --reset-demo
```

Use a live cache with a slower refresh:

```bash
stock-tui --feed iex --refresh-seconds 900
```

Inspect a cache without network access:

```bash
stock-tui --offline --db /private/path/market.sqlite3
```

Diagnose configuration without entering the terminal UI:

```bash
stock-tui --print-config
```
