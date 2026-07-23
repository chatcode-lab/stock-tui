# Data Providers And Licensing

`stock-tui` combines issuer identity, market observations, and news. These
sources have different accuracy guarantees and legal terms. The MIT license on
the client does not grant rights to third-party data.

This document summarizes the current design, not legal advice. Provider plans
and terms change; verify them for your account and use case.

## Source Matrix

| Data | Current source | Stored locally | Notes |
| --- | --- | --- | --- |
| Nine-sector candidates and proxy rank | Versioned catalog generated from SEC identity, SIC, and XBRL facts | Yes | Keeps 100-250 candidates per sector and displays the selected top 100. |
| Issuer name, ticker, exchange associations | US SEC EDGAR catalog, supplemented by Alpaca active assets | Yes | Associations are identifiers, not a complete security master. |
| Current price, previous close, OHLC, volume | Alpaca stock snapshots | Yes | Coverage depends on the selected feed and subscription. |
| Historical OHLCV, trades, VWAP | Alpaca multi-symbol bars | Yes | Requests use `adjustment=all`; five-year bulk cache uses daily bars. |
| News headline, date, source, summary, URL, symbols | Alpaca Historical News API (currently Benzinga content) | Yes | Loaded lazily for an opened ticker. |
| Demo issuer identities | Embedded SEC-derived catalog | Yes | Real ticker/name associations; not a claim that the security remains active. |
| Demo prices, rankings, volume, descriptions, news | Built-in deterministic generator | Yes | Entirely simulated and visibly labeled; no provider market data is used. |

## Alpaca

The live adapter calls Alpaca's Trading API for active US equity asset names
and exchanges and its Market Data API for snapshots, adjusted bars, and news.
Requests authenticate with the key and secret belonging to the local user.
Credentials are not bundled with the project and must not be submitted in bug
reports.

The current scope is US equities only. `feed` selects IEX/SIP behavior inside
that scope; it is not a region switch. Non-US instruments require a separate
adapter that defines symbols, currencies, calendars, corporate actions,
entitlements, and licensing.

Relevant official documentation:

- [About the Market Data API](https://docs.alpaca.markets/us/docs/about-market-data-api)
- [Market Data FAQ](https://docs.alpaca.markets/us/docs/market-data-faq)
- [Historical stock data](https://docs.alpaca.markets/us/docs/historical-stock-data-1)
- [Historical news data](https://docs.alpaca.markets/us/docs/historical-news-data)
- [News endpoint reference](https://docs.alpaca.markets/us/reference/news-3)
- [Alpaca disclosures and agreements](https://alpaca.markets/disclosures)

### Basic Plan And IEX

Alpaca currently documents its individual Basic plan as free, with US stocks
and ETFs, real-time equity coverage from IEX, historical data since 2016, a
restriction on the latest 15 minutes for historical SIP data, and 200
historical API calls per minute. IEX is one exchange, so IEX trade counts,
volume, OHLC, and last prices can differ materially from consolidated SIP
figures across all US exchanges.

The client therefore defaults to:

- `feed = "iex"`
- 180 requests per minute, leaving headroom below the documented Basic limit
- 100 symbols per snapshot request
- 50 symbols per historical-bars request
- a five-minute snapshot refresh cadence

These are client-side limits, not a promise that an account is entitled to a
request. Alpaca remains authoritative. The adapter handles pagination, retries
transient failures, and reports authentication/permission errors without
falling back to fabricated live values.

`sip` requires appropriate account entitlement for current consolidated data.
When a requested snapshot feed is unavailable, the adapter may try an allowed
fallback and ultimately IEX. `delayed_sip` uses SIP historical bars ending 16
minutes before the current time and allows snapshot fallback behavior, but
exact availability is account-dependent.

### Redistribution

Alpaca's official support page states plainly that customers cannot
redistribute Alpaca API data:
[Can I redistribute Alpaca API data via my platform?](https://alpaca.markets/support/redistribute-alpaca-api).
Its published agreements impose additional market-data conditions.

Consequences for this project:

- The open-source repository contains code and simulated demo data, not a
  populated Alpaca database.
- A user's cache is for that credential holder's authorized local use.
- Do not commit, attach to a release, mirror, sell, or serve a populated Alpaca
  cache under ordinary API terms.
- Do not use a personal key as a shared proxy for other users.
- Anyone operating a public service must obtain written rights appropriate to
  its display, redistribution, retention, geography, and user classes.

The planned no-key fallback backend cannot launch merely by moving the current
cache to a server. It requires separately licensed market data and news whose
agreements explicitly allow the intended redistribution. The service must also
preserve required attribution and delay labels and prevent extraction beyond
its licensed scope.

## News

The client requests the 20 newest items related to a ticker from Alpaca's
`/v1beta1/news` endpoint, without article body content. It stores provider ID,
headline, source, publication time, URL, summary, and related symbols. The TUI
shows the concise date, headline, and source; activating a row opens the
publisher/provider URL in the default browser. When browser launch is
unavailable, it copies that URL through OSC 52 so a browser-hosted terminal can
offer it to the client clipboard.

Alpaca documents historical news back to 2015 and identifies Benzinga as its
current news source. Availability, permitted display, retention, and
attribution remain governed by the user's Alpaca and content-provider terms.
News may be duplicated, revised, misclassified, unavailable, or unrelated to a
ticker despite the symbol association. It is not research or a recommendation.

Demo headlines use invalid example URLs and explicitly identify their headline,
source, and summary as simulated. The TUI also keeps a `SIMULATED` badge visible
while demo data is active.

## SEC-Derived Issuer Catalog

The embedded [`data/sec_universe.json`](../data/sec_universe.json) is generated
entirely from official SEC sources:

- [`company_tickers_exchange.json`](https://www.sec.gov/files/company_tickers_exchange.json)
  supplies CIK, EDGAR conformed name, ticker, and exchange associations.
- The SEC's quarterly
  [Financial Statement Data Sets](https://www.sec.gov/data-research/sec-markets-data/financial-statement-data-sets)
  supply the most recently filed Standard Industrial Classification (SIC) for
  an issuer.
- The SEC XBRL
  [Frames API](https://www.sec.gov/search-filings/edgar-application-programming-interfaces)
  supplies `dei:EntityPublicFloat` in USD and, when reported,
  `dei:EntityCommonStockSharesOutstanding` in shares.

The JSON records its schema/catalog versions, generation and as-of timestamps,
selection method, source URLs and retrieval times, and fact-level accession/
frame provenance. Embedding a reviewed snapshot makes releases reproducible
and keeps runtime startup independent of SEC availability.

The checked-in JSON preserves the SEC's hyphen notation for share classes.
When loading the catalog, the client converts those symbols to Alpaca notation
before validation or provider requests: `BRK-B` becomes `BRK.B`, while an SEC
preferred-share suffix such as `TRTN-PA` becomes Alpaca's `TRTN.PRA`.

The checked-in schema-v1 catalog contains 1,880 unique CIK/canonical-symbol
candidates, with 103 to 250 candidates per sector. Those counts describe this
catalog revision, not a guaranteed future universe size. No Nasdaq data service
is used to construct it; `Nasdaq` appears only as an exchange label supplied by
the SEC association file.

### Selection Pipeline

The catalog builder:

1. Keeps SEC associations on NYSE, Nasdaq, or CBOE with an ASCII ticker.
2. Chooses one deterministic canonical ticker for each CIK, preferring a symbol
   that does not look like a preferred, warrant, unit, or right suffix, then
   preserving SEC file order.
3. Takes the newest SIC observation from the requested recent Financial
   Statement Data Set quarters.
4. Searches recent quarterly XBRL frames for positive public float and optional
   shares outstanding facts.
5. Rejects non-finite/non-positive facts, extreme absolute float values,
   implausible float-to-share ratios, and an isolated newest public-float fact
   more than 100 times above or below prior observations.
6. Maps SIC to the nine legacy display sectors, ranks each sector by reported
   public float descending, deduplicates symbols, and retains between 100 and
   250 eligible candidates per sector.

`EntityPublicFloat` is a filer-reported issuer-level value and is **not market
capitalization**. The build does not write it into `Company.market_cap`; it is
only the initial ranking proxy. When both SEC-reported shares and an Alpaca
snapshot price exist, runtime estimates market cap as shares times current
price. Each successful candidate snapshot refresh then selects 100 companies
per sector by known estimated market cap, with proxy rank as fallback. Only
those 900 companies receive the bulk five-year history backfill.

This means a company can move into the visible top 100 as prices change if it
is already in the embedded candidate pool and has usable shares. A new issuer,
a newly eligible filer, or a company outside that pool requires a catalog
rebuild and project release. Public-float and shares facts can have different
as-of dates, and a missing share fact leaves membership dependent on the proxy.

Alpaca's active-asset response refreshes names and exchange identifiers for
symbols it recognizes without overwriting SEC-derived SIC sector, proxy rank,
shares, or retention state. Alpaca-only active symbols remain searchable and
can load detail, but do not enter a sector without catalog metadata.

### Quality Limits

The SEC explicitly says its ticker association files are periodically updated
and that it does not guarantee their accuracy or scope. Its Financial Statement
Data Sets are derived from filer submissions, may contain extraction errors,
omit some filing metadata, and are not a substitute for full filings. See
[Accessing EDGAR Data](https://www.sec.gov/search-filings/edgar-search-assistance/accessing-edgar-data#cik-ticker-associations)
and the SEC's Financial Statement Data Set disclaimer.

Consequently:

- An association does not prove a security is active, liquid, primary-listed,
  common stock, or available from Alpaca.
- One canonical symbol per CIK necessarily omits other share classes and can
  still select a non-common instrument despite the suffix heuristic.
- Tickers and names can change, be reused, or have inconsistent punctuation
  between SEC and market-data systems.
- SIC is an issuer classification, not a security-level modern sector
  taxonomy, and the project's nine-sector mapping is heuristic.
- `EntityPublicFloat` and shares outstanding are self-reported point-in-time
  facts with issuer-specific filing practices; screening catches only obvious
  anomalies.
- Shares can be absent or ambiguous for multi-class issuers, and a current SEC
  ticker identity can temporarily fail to join older facts after a CIK
  reorganization.
- Depositary receipts, funds, warrants, units, rights, test symbols, and foreign
  issuers can require handling that these general filters do not capture.
- EDGAR's conformed issuer name is not necessarily the consumer-facing brand.

The TUI is a broad visual catalog, not an index product or authoritative
security master.

### Catalog Build Process

The maintenance-only builder is
[`tools/build_sec_catalog.py`](../tools/build_sec_catalog.py). It uses Python's
standard library and is not run by the Rust application or included as a
runtime dependency. A maintainer supplies a truthful SEC contact identity:

```bash
python3 tools/build_sec_catalog.py \
  --user-agent "stock-tui catalog maintainer@example.invalid" \
  --through 2026Q1
cargo test universe
```

The tool accepts `SEC_USER_AGENT` instead of `--user-agent`, caches source
downloads under `~/.cache/stock-tui/sec-catalog` by default, restricts requests
to `www.sec.gov` and `data.sec.gov`, and defaults to eight requests per second.
It refuses a setting above the SEC's current aggregate maximum of ten requests
per second, retries transient failures, writes source receipts, validates
unique CIKs/symbols and consecutive per-sector ranks, and atomically replaces
the output.

Catalog updates must review the JSON diff, source dates, sector counts, large
rank movements, and quality labels before commit. Reusing the download cache
improves reproducibility but maintainers should deliberately choose when a
fresh SEC retrieval is required.

The builder follows the SEC's
[Developer Resources and Fair Access guidance](https://www.sec.gov/about/developer-resources).
Runtime market refreshes read the embedded file and do not poll SEC.

## Nine-Sector Legacy Taxonomy

The visualization intentionally uses the nine groups from the StockTouch-era
experience rather than claiming compatibility with today's 11-sector GICS
model. The SEC catalog maps SIC ranges using explicit precedence in the build
tool. In broad terms, extractive/oil SICs map to Energy; regulated utility SICs
to Utilities; finance and real-estate SICs to Financial; healthcare services,
pharma, and medical-device SICs to Healthcare; computing/electronics/software
SICs to Technology; mining/forestry/paper/chemicals/metals to Materials;
agriculture, food, apparel, household and selected vehicle/recreation SICs to
Consumer; construction/manufacturing/transportation to Industrial; and trade,
communications, media, hospitality, and professional-service SICs to Services.
Unmatched SICs currently fall back to Industrial and should be reviewed during
catalog updates.

The domain also normalizes future provider text labels as follows:

| `stock-tui` sector | Accepted provider families |
| --- | --- |
| Consumer | Consumer, Consumer Cyclical, Consumer Defensive, Consumer Discretionary, Consumer Durables, Consumer Non-Durables, Consumer Staples |
| Services | Services, Communication Services, Miscellaneous, Telecommunications |
| Healthcare | Health Care, Healthcare |
| Energy | Energy |
| Technology | Technology |
| Financial | Finance, Financial Services, Financials, Real Estate |
| Industrial | Capital Goods, Industrial, Industrials |
| Materials | Basic Industries, Basic Materials, Materials |
| Utilities | Utilities |

Unknown provider text labels remain unclassified rather than being guessed.
Notable legacy collapses include Real Estate into Financial and Communication
Services into Services. Sector returns are visualization aggregates, not
published indexes.

## Market-Cap Ranking Quality

The heatmap can order by market capitalization, but neither Alpaca's basic
asset payload nor the SEC source set is a complete fundamentals feed. The SEC
public-float proxy and shares facts are point-in-time metadata and can become
stale between catalog releases. When shares are available, a new snapshot
estimates market cap as shares times price; this remains an approximation and
does not handle every share class, treasury-share treatment, corporate action,
or float convention.

Missing market caps sort after known values. Gain and volume ordering use the
selected cached period/snapshot. Alphabetical ordering uses ticker symbol.

## Adding A Provider

A provider contribution must include:

1. Official provenance and API documentation.
2. Written analysis of personal display, caching, retention, attribution, and
   redistribution rights.
3. Exact feed coverage, delay, corporate-action adjustment, timezone, and
   symbol semantics.
4. Secret-safe configuration and redacted errors.
5. Pagination, timeout, retry, and rate-limit behavior.
6. Fixture-based tests that never call a paid or credentialed service.
7. A mapping strategy that leaves unknown sectors and instruments explicit.

Do not add scraping of a website that forbids automated access or data reuse.
An API being technically reachable does not make its data redistributable.
