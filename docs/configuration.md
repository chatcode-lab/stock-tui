# Configuration

`stock-tui` has command-line flags, environment variables, a local dotenv
file, and a strict TOML file. Credentials are accepted only from the
environment.

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

## Credentials

Both variables must be set together and neither may be empty:

| Environment variable | Purpose |
| --- | --- |
| `ALPACA_API_KEY` | The local user's Alpaca API key ID. |
| `ALPACA_API_SECRET` | The matching Alpaca API secret. |

The default Trading API endpoint is Alpaca's paper environment, so free paper
account credentials work without a funded live brokerage account. `stock-tui`
uses that endpoint only to read the active US-equity asset directory; it does
not submit or manage orders. Keys issued for a different Alpaca environment
must be paired with the corresponding `STOCK_TUI_TRADING_URL` override.

If both are absent, the application selects demo mode unless `--offline` was
explicitly requested. If only one is set, startup fails instead of making a
partially authenticated request.

The dotenv loader looks for `.env` from the current working directory using
standard dotenv discovery. Keep that file private and outside version control.
Do not put credentials in `config.toml`, command history, screenshots, issues,
or release assets.

## Command-Line Flags

| Flag | Meaning |
| --- | --- |
| `--demo` | Use the deterministic simulated market, even if credentials exist. |
| `--reset-demo` | Clear the entire selected database and rebuild demo records; requires `--demo`. |
| `--offline` | Never start remote synchronization; render the selected cache. |
| `--db <PATH>` | Override the SQLite database path. |
| `--feed <FEED>` | Select `iex`, `delayed_sip`, or `sip`. |
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
| `STOCK_TUI_DB_PATH` | Platform data dir plus `market.sqlite3` | Equivalent to `--db`. |
| `STOCK_TUI_FEED` | `iex` | `iex`, `delayed_sip`, or `sip`; entitlement remains provider-controlled. |
| `STOCK_TUI_REFRESH_SECONDS` | `300` | Equivalent to `--refresh-seconds`; clamped to 30..86,400. |
| `STOCK_TUI_DATA_URL` | `https://data.alpaca.markets` | Alpaca Market Data base URL; mainly for controlled testing/proxies. |
| `STOCK_TUI_TRADING_URL` | `https://paper-api.alpaca.markets` | Alpaca paper Trading API base URL, used only for asset metadata. |
| `NO_COLOR` | Unset | Any value selects the monochrome heat palette. |
| `RUST_LOG` | `stock_tui=info,warn` | Tracing filter for daily files below `<cache_dir>/logs`. |

Changing service URLs sends credentials to those hosts. Non-loopback provider
URLs must use HTTPS; plain HTTP is accepted only for local fixture servers.
Only point a live build at infrastructure you trust and control. URL overrides
do not waive provider terms or create redistribution rights.

## TOML File

The file is `config.toml` in the platform configuration directory. Find the
exact `config_dir` with `--print-config`.

```toml
feed = "iex"
refresh_seconds = 300
request_limit_per_minute = 180
snapshot_batch_size = 100
history_batch_size = 50

# Advanced provider endpoints:
# data_url = "https://data.alpaca.markets"
# trading_url = "https://paper-api.alpaca.markets"
```

Supported keys and validation:

| Key | Default | Accepted value |
| --- | --- | --- |
| `feed` | `iex` | `iex`, `delayed_sip`, or `sip` |
| `refresh_seconds` | `300` | Integer, clamped to 30..86,400 |
| `request_limit_per_minute` | `180` | Integer, clamped to 1..200 |
| `snapshot_batch_size` | `100` | Integer, clamped to 1..500 |
| `history_batch_size` | `50` | Integer, clamped to 1..200 |
| `data_url` | Alpaca production data URL | HTTPS base URL, or loopback HTTP for tests |
| `trading_url` | Alpaca paper trading URL | HTTPS base URL, or loopback HTTP for tests |

Credentials and the database path are intentionally absent from TOML. Use the
environment and `--db` / `STOCK_TUI_DB_PATH` respectively.

The runtime uses the catalog embedded at compile time; SEC URLs and identity
are not runtime settings. Catalog maintainers run
`tools/build_sec_catalog.py` separately with `--user-agent` or the
`SEC_USER_AGENT` build-tool environment variable. See
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
