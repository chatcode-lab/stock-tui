# Contributing To stock-tui

Thank you for improving `stock-tui`. Contributions can include bug reports,
terminal compatibility notes, documentation, tests, accessibility work,
performance fixes, provider adapters, and focused UI changes.

By participating, you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
Security-sensitive reports belong in the private process described in
[SECURITY.md](SECURITY.md), not a public issue.

## Before Starting

Open an issue before substantial work that changes behavior, storage schema,
the nine-sector taxonomy, data provenance, or provider licensing. A short
design discussion avoids parallel incompatible implementations. Small bug
fixes, tests, and documentation corrections can go directly to a pull request.

When evaluating a change, maintainers prioritize:

- Correctness and explicit data semantics over apparent precision.
- A useful cached/offline path over a network-dependent first screen.
- Mouse and keyboard parity.
- Readability at both compact and full terminal sizes.
- Bounded provider usage and secret-safe diagnostics.
- Clear provenance and legal permission for every third-party field.

## Development Setup

The minimum supported Rust version is 1.95. Install Rust through
[rustup](https://rustup.rs/), then clone and test the project:

```bash
git clone https://github.com/chatcode-lab/stock-tui.git
cd stock-tui
rustup show
cargo build --locked
cargo test --all-targets --all-features
```

Run with generated data during normal development:

```bash
cargo run -- --demo --db /tmp/stock-tui-development.sqlite3
```

Use a private, platform-appropriate temporary path on systems without `/tmp`.
The demo is deterministic and covers 900 sector companies plus three benchmark
ETF proxies, all ten date ranges, charts, news, search, and favorites without
consuming API quota.
`--reset-demo` is deliberately destructive to its selected database, so never
point that command at a cache or favorites list you need to retain.

Real Alpaca credentials are not needed for the test suite. Never add them to a
fixture, shell transcript, screenshot, issue, pull request, or CI setting for a
fork you do not control.

## Required Checks

Run these before opening a pull request:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release --locked
```

Avoid unrelated lockfile or formatting churn. The repository forbids unsafe
Rust. New dependencies need a concrete benefit and should not duplicate an
existing capability.

## Manual TUI Checks

Changes to application state, rendering, input, or data loading also need a
short manual pass:

1. Start `--demo` in a terminal at least 120x36.
2. Resize to compact mode around 80x24 and then below 60x20.
3. Navigate Overview -> Sector -> Ticker -> Back with both mouse and keyboard.
4. Exercise every date range and ordering mode.
5. Search by an exact ticker and a partial company name.
6. Star a ticker, confirm its marker in common views, open Starred, then unstar
   it.
7. Hover and keyboard-scrub the Braille chart; switch compact detail tabs.
8. Open and close every overlay and quit with both `q` and `Ctrl-C`.
9. Confirm the terminal cursor, mouse mode, raw mode, and alternate screen are
   restored after normal exit and an induced recoverable error.
10. Check `NO_COLOR=1` and at least one terminal/font combination different
    from your primary environment when the palette or glyphs change.

Include tested terminal names, operating systems, and viewport sizes in the
pull request for rendering changes. Do not attach screenshots containing live
provider data unless your provider agreement and account permit publication;
prefer demo mode.

## Tests

Match coverage to the behavior being changed:

- Domain mapping: table-driven unit tests for every accepted and rejected
  label.
- Storage: temporary file-backed SQLite databases, atomic batch assertions,
  migration/version checks, and restart behavior.
- Provider: controlled local HTTP fixtures covering authentication headers,
  pagination, fallbacks, retries, errors, and redaction. Never contact a real
  provider in tests.
- Sync: deterministic clocks/watermarks where practical and assertions that a
  partial failure preserves completed cache work.
- UI: layout invariants, Unicode width, hit targets, keyboard/mouse command
  parity, and endpoint-preserving chart sampling.

Financial calculations should have explicit examples for missing values,
zero baselines, cutoff boundaries, stale timestamps, and non-finite provider
input.

## Provider And Catalog Contributions

Do not submit a provider merely because its endpoint is accessible. A provider
pull request must document:

- An official source and API reference.
- Authentication and secret-handling requirements.
- Rate, pagination, retry, and timeout behavior.
- Venue coverage, delay, timezone, currency, adjustment, and symbol semantics.
- Rights for local display, caching, retention, attribution, and
  redistribution.
- A plan for delistings, symbol changes, share classes, and malformed data.
- Fixture-based tests with synthetic responses.

The MIT license does not make third-party data open. Do not commit a populated
Alpaca cache or provider-derived corpus. Alpaca says ordinary API data cannot
be redistributed; see [Data Providers](docs/data-providers.md).

Catalog updates must preserve source provenance and generation date, explain
ranking and sector inputs, and call out manual overrides. The SEC
CIK/ticker/exchange association file does not itself contain sector or market
cap and does not guarantee accuracy or scope. Unknown mappings must remain
unknown instead of being silently forced into a sector.

The nine-sector model is an intentional StockTouch-era product decision. A
proposal to adopt another taxonomy needs a migration and compatibility design,
not only renamed labels.

## Storage Changes

Never rewrite an existing migration after release. Add a forward migration,
increment `PRAGMA user_version`, and test:

- opening a new database;
- upgrading the prior version with data intact;
- rejecting a database newer than the binary;
- foreign key and WAL behavior;
- concurrent read/write access where relevant.

Avoid destructive cleanup during startup. Retained and favorited companies may
intentionally remain outside the current universe.

## Documentation

Update documentation in the same pull request when changing flags, settings,
controls, providers, cache behavior, schema, limits, responsive breakpoints, or
financial semantics. Use `chatcode-lab` (singular) in repository URLs.

External claims about current plans, limits, terms, or APIs need a direct link
to an official source and a date-sensitive formulation such as "currently".
Do not paste credentials or real secret-looking examples; use clearly fake
placeholders.

## Pull Requests

Keep pull requests focused. The description should state:

- The user-visible problem and chosen behavior.
- Important alternatives or compatibility tradeoffs.
- Tests run and manual terminal matrix.
- Storage/provider/licensing impact, even when the answer is "none".
- Follow-up work that remains intentionally out of scope.

Use clear commit messages in the imperative mood, for example
`Handle missing snapshot baselines`. Maintainers may ask for commits to be
squashed before merge.

By submitting a contribution, you agree that it may be distributed under the
repository's [MIT License](LICENSE) and affirm that you have the right to submit
it.
