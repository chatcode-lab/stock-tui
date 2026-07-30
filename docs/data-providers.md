# Data Providers And Licensing

`stock-tui` combines issuer identity, market observations, and news. These
sources have different accuracy guarantees and legal terms. The MIT license on
the client does not grant rights to third-party data.

This document summarizes the current design, not legal advice. Provider plans
and terms change; verify them for your account and use case.

## Source Matrix

| Data | Current source | Stored locally | Notes |
| --- | --- | --- | --- |
| Nine-sector candidates, SIC industry labels, size proxy, and common-share estimates | Versioned catalog generated from SEC identity, SIC taxonomy, and XBRL facts, published as compact JSON through Cloudflare R2 | Yes | Keeps 100-250 candidates per sector, with a validated embedded release fallback, and displays the selected top 100. |
| Issuer name, ticker, exchange associations | US SEC EDGAR catalog, supplemented by Alpaca active assets | Yes | Associations are identifiers, not a complete security master. |
| Overview benchmarks | Alpaca stock data for `SPY`, `DIA`, and `QQQ` ETF proxies | Yes | Labeled as proxies; values are not literal S&P 500, Dow, or Nasdaq index levels. |
| Current price, previous close, OHLC, volume | Alpaca stock snapshots | Yes | Coverage depends on the selected feed and subscription. |
| Historical OHLCV, trades, VWAP | Alpaca multi-symbol bars | Yes | Requests use `adjustment=all`; the bulk cache keeps two years of daily bars and all available weekly bars. |
| Forward and reverse stock splits | Selected provider corporate-actions capability; Alpaca Market Data API in the default adapter | Derived cap only | Reconciles dated SEC share estimates with current prices; raw split events are not persisted. |
| News headline, date, source, summary, URL, symbols | Alpaca Historical News API (currently Benzinga content) | Yes | Loaded lazily for an opened ticker. |
| Demo issuer identities | Embedded SEC-derived catalog | Yes | Real ticker/name associations; not a claim that the security remains active. |
| Demo prices, rankings, volume, descriptions, news | Built-in deterministic generator | Yes | Entirely simulated and visibly labeled; no provider market data is used. |

Live company context is deliberately concise: it combines the SEC issuer name,
listing exchange, SIC code, and, when present in the selected catalog, the SIC
taxonomy's official industry label. It is a classification summary, not a
provider-supplied business profile.

The source matrix describes the default Alpaca configuration. The separately
selectable `stock-api` adapter consumes normalized observations from an
operator-supplied service; its actual sources, coverage, delays, and rights are
properties of that deployment.

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
- [Corporate actions endpoint](https://docs.alpaca.markets/us/reference/corporateactions-1)
- [Historical news data](https://docs.alpaca.markets/us/docs/historical-news-data)
- [News endpoint reference](https://docs.alpaca.markets/us/reference/news-3)
- [Index market-data launch](https://docs.alpaca.markets/us/changelog/2026-06-03-market-data-9dddd18)
- [Alpaca disclosures and agreements](https://alpaca.markets/disclosures)

### Benchmark Proxies

The overview uses `SPY` for the S&P 500, `DIA` for the Dow Jones Industrial
Average, and `QQQ` for the Nasdaq-100. These liquid ETFs are explicitly labeled
as proxies and use the same free-equity snapshot, bars, and news paths as other
tickers, so each cell can open a complete detail view.

Alpaca also exposes native index-value endpoints, but access depends on an
account's index-data entitlement and those value records do not provide the
same OHLCV/news contract as the stock endpoints. The client therefore does not
silently substitute native index levels for ETF prices. A future provider can
add entitled native indices as a separately labeled instrument type.

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

At live startup, the client reconciles resolved sector candidates against
Alpaca's active-asset response before requesting snapshots. Missing candidates
are excluded from current memberships and routine snapshot refresh, but their
company rows, favorites, and cached observations are preserved. A later
active-asset response reactivates catalog candidates. Alpaca's `active` status
does not by itself guarantee current liquidity, tradability for a particular
account, or complete quote coverage.

Alpaca can emit flat zero-volume daily rows while a security has no trades,
including during a halt. The client preserves those rows in its raw cache but
does not use a row with zero or absent trades and identical OHLC prices as a
price observation, freshness timestamp, history endpoint, or chart point.

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

The project therefore uses a bring-your-own-key model. A no-key backend cannot
launch merely by moving the current cache to a server. It would require
separately licensed market data and news whose agreements explicitly allow the
intended redistribution. The service would also need to preserve required
attribution and delay labels and prevent extraction beyond its licensed scope.

### Requesting Public-Display Permission

Alpaca documents a
[partnership inquiry](https://alpaca.markets/support/partner-with-alpaca) for
applications that need terms beyond ordinary personal API access. Use that
form, or an authenticated Alpaca support request, and retain the complete
written response. Thirty days' advance notice under the general terms is not
the same as redistribution permission.

Ask for explicit written answers for every intended use:

- Whether a free or sponsored open-source license exists, and any user,
  geography, traffic, or non-commercial limits.
- Public display and redistribution of IEX snapshots, adjusted historical
  bars, volumes, and derived returns or heatmap colors.
- Server-side caching, permitted retention periods, refresh delays, and whether
  an endpoint may serve arbitrary unauthenticated users.
- Whether raw downloads must be prevented and which attribution, disclaimer,
  audit, reporting, or deletion controls are required.
- Separate permission for news headlines, summaries, URLs, source names, and
  symbol associations. Alpaca may not be able to grant rights owned by its news
  supplier.

A concise inquiry can use this structure:

> Subject: Open-source market-data display and redistribution permission for
> stock-tui
>
> I maintain stock-tui, an MIT-licensed, non-commercial terminal application:
> https://github.com/chatcode-lab/stock-tui. Today each user supplies their own
> Alpaca key and keeps data in a local SQLite cache. We will not operate a
> shared-key service without written permission.
>
> Is a free, sponsored, or low-cost license available for this open-source
> project to display and cache Alpaca-provided US equity data for public users?
> The proposed fields are snapshots, adjusted OHLCV bars, derived percentage
> changes and heatmap colors, plus date/headline/source/URL news metadata. Please
> specify whether permission covers IEX and/or SIP data, derived displays,
> server-side caching and retention, unauthenticated users, global access, and
> news-provider content; also list required delays, attribution, extraction
> controls, reporting, and audience limits.
>
> If Alpaca cannot grant these rights, please confirm which underlying
> licensors must be contacted. We would need an agreement that expressly
> permits public display and redistribution before enabling such a service.

Do not interpret a marketing reply, rate-limit increase, API plan upgrade, or
OAuth approval as a market-data license. The response must specifically cover
the contemplated fields and distribution model.

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

## Provider-Neutral Stock API

The `stock-api` adapter is an optionally bearer-authenticated HTTP client for
assets, snapshots, adjusted bars, and optional news. It is selected explicitly
with `--provider stock-api` or `provider = "stock-api"` and a
provider-specific base URL. It never sends Alpaca keys, cookies, or generic
API-key headers. When `STOCK_TUI_STOCK_API_TOKEN` or the lower-precedence
`providers.stock_api.token` setting is present, it sends that secret only as a
bearer token through `StockApiProvider`; when both are unset it sends no
authorization header.

The routes and payloads do not name or assume an upstream vendor. They include
generic `source` fields where provenance must survive normalization. The full
request/response schema, pagination, validation, timeout, body limits, errors,
and cache semantics are specified in
[Stock API HTTP Contract](stock-api-contract.md).

`https://stock.chatcode.dev/api` is the private project development endpoint
and configuration default, not a licensed public market-data service. It
requires an out-of-band bearer token for authorized remote tests. A local
Cloudflare Worker can be tested at `http://127.0.0.1:8787`; other non-loopback
deployments must use HTTPS.

This adapter is an interoperability boundary, not a redistribution loophole.
The operator must have written rights for every upstream field and intended
user, display, cache, retention period, geography, and news use. Required
source attribution must remain accurate. A compatible JSON response does not
prove that its data was lawfully obtained or may be redistributed.

## SEC-Derived Issuer Catalog

The reviewed fallback [`data/sec_universe.json`](../data/sec_universe.json) and
the compact remote catalog are generated entirely from official SEC sources:

- [`company_tickers_exchange.json`](https://www.sec.gov/files/company_tickers_exchange.json)
  supplies CIK, EDGAR conformed name, ticker, and exchange associations.
- The SEC's quarterly
  [Financial Statement Data Sets](https://www.sec.gov/data-research/sec-markets-data/financial-statement-data-sets)
  supply the most recently filed Standard Industrial Classification (SIC) and
  standard-taxonomy common-share facts for an issuer.
- The SEC's annual
  [SIC taxonomy](https://www.sec.gov/search-filings/standard-industrial-classification-sic-code-list)
  supplies the official short industry label for each SIC code.
- The SEC XBRL
  [Frames API](https://www.sec.gov/search-filings/edgar-application-programming-interfaces)
  supplies `dei:EntityPublicFloat` in USD and, when reported,
  `dei:EntityCommonStockSharesOutstanding` in shares, plus explicitly reviewed
  issuer aggregates such as `us-gaap:CommonStockSharesOutstanding`.
- The SEC
  [Submissions API](https://www.sec.gov/search-filings/edgar-application-programming-interfaces)
  identifies each affected issuer's latest inline 10-K, 10-Q, 20-F, or 40-F.
  The builder reads that filing's immutable extracted XBRL instance so cover
  shares retain their exact class dimensions.

The audit JSON records its schema/catalog versions, generation and as-of
timestamps, selection method, source URLs and retrieval times, and fact-level
accession/frame provenance. Packaging projects it onto the fields consumed by
the Rust client, serializes canonical JSON, and produces deterministic gzip
plus a checksum manifest. The current compact catalog is served at
`https://stock.chatcode.dev/catalog/sec-catalog.json`; it contains SEC-derived
issuer metadata, not provider prices, bars, volume, or news.

Every release still embeds a validated compact snapshot. At runtime a
low-frequency background check can replace it with a newer validated R2
catalog in the local cache, while network, unsupported-schema, unsafe-text,
duplicate, rank, provenance, size, and downgrade failures fall back to the
newest valid local or embedded copy. The application never contacts SEC
directly.

The checked-in JSON preserves the SEC's hyphen notation for share classes.
When loading the catalog, the client converts those symbols to Alpaca notation
before validation or provider requests: `BRK-B` becomes `BRK.B`, while an SEC
preferred-share suffix such as `TRTN-PA` becomes Alpaca's `TRTN.PRA`.

The checked-in schema-v2 catalog contains 1,880 unique
CIK/canonical-symbol candidates, with 102 to 250 candidates per sector. Those
counts describe this catalog revision, not a guaranteed future universe size.
No Nasdaq data service is used to construct it; `Nasdaq` appears only as an
exchange label supplied by the SEC association file.

### Selection Pipeline

The catalog builder:

1. Keeps SEC associations on NYSE, Nasdaq, or CBOE with an ASCII ticker.
2. Chooses one deterministic canonical ticker for each CIK, preferring a symbol
   that does not look like a preferred, warrant, unit, or right suffix, then a
   valid base symbol of four or fewer characters when the source-preferred
   unseparated sibling shares that prefix, then preserving SEC file order.
   Explicit share-class suffixes remain source-selected because classes can
   have different per-share economics. The reviewed issuer-specific exception
   is Molson Coors (CIK `0000024545`): `TAP` Class B is selected over `TAP-A`
   Class A because the classes share dividend and undistributed-earnings
   economics, Class A converts one-for-one into Class B, and Class B is the
   substantially larger listed class. Their voting rights still differ, so
   this economic/liquidity choice does not relax the general share-class rule.
   When a newer catalog changes an issuer's canonical symbol, the client
   retires the previous symbol from sector membership without aliasing the two
   securities' prices, bars, or favorites.
3. Takes the newest SIC observation from the requested recent Financial
   Statement Data Set quarters and joins its official SEC taxonomy label.
4. Searches recent quarterly XBRL frames, independently of the newest available
   bulk-file quarter, for positive public float, unsegmented DEI share totals,
   and issuer-scoped reviewed US-GAAP common-share totals. It rejects malformed
   or future-dated frame observations. It also scans Financial Statement Data
   Set `num` rows for eligible standard-taxonomy share facts from 10-K, 10-Q,
   20-F, and 40-F filings.
5. For every unresolved, partnership-fallback, or reviewed issuer, resolves the
   latest eligible inline filing from SEC submissions and parses its extracted
   XBRL instance. This preserves the exact
   `StatementClassOfStockAxis`/`ClassOfStockAxis` members that aggregate APIs
   can flatten.
6. Selects a price-equivalent share basis using the reviewed hierarchy below
   and stores its accession, source, fact date, method, confidence, components,
   multipliers, policy basis, and SEC filing URL. A fact more than 550 days old
   is rejected even when no newer candidate exists.
7. Rejects non-finite/non-positive facts, extreme absolute float values,
   grossly inconsistent public-float/filer-status combinations, an unreviewed
   implied float-per-share above `$2,000`, and an isolated newest public-float
   fact more than 100 times above prior observations. Downward corrections and
   reviewed legitimate high-price issuers are retained. A float above
   `$70 billion` with no accelerated-filer status also requires explicit
   review.
8. Maps SIC to the nine legacy display sectors, ranks each sector by reported
   public float descending, deduplicates symbols, and retains between 100 and
   250 eligible candidates per sector. It emits coverage and unresolved-reason
   metadata and refuses the build if any current sector top-100 candidate lacks
   a share basis.

The share hierarchy is deliberately fail-closed:

1. A latest-filing fact set whose exact class signature matches a reviewed
   policy. Each expected cover member has an explicit multiplier, including
   zero for a reported non-economic or out-of-scope class. A policy may also
   combine exact issuer-reported economic-unit or fully exchanged share facts,
   pinned to an accession, namespace, period, unit, and dimension signature.
   Unknown, missing, renamed, or inconsistent inputs invalidate the policy.
2. A latest-filing unsegmented or single unambiguous common class, high
   confidence. Preferred, warrant, right, option, debt, redeemable, and
   temporary-equity members are excluded.
3. An unsegmented `dei:EntityCommonStockSharesOutstanding` total or a reviewed
   sum of DEI common classes, high confidence.
4. An unsegmented `us-gaap:CommonStockSharesOutstanding` issuer total, a
   reviewed sum of equal-economic classes, or a reviewed class conversion,
   medium confidence.
5. A filer-reported equivalent class, unsegmented
   `us-gaap:WeightedAverageNumberOfSharesOutstandingBasic`, or
   `us-gaap:WeightedAverageLimitedPartnershipUnitsOutstanding`, low confidence.

Automatic cover and class-member calculations use point-in-time facts.
An accession-scoped reviewed policy can deliberately use a cited duration fact,
such as a filer-reported fully exchanged weighted average, and records the
lower confidence and scope explicitly. Generic basic weighted average remains
a lower-quality fallback; diluted shares, preferred shares, RSUs, options, and
unexercised convertibles are not inferred. When the newest generic fact is
point-in-time, an older higher-confidence point fact can override it only
within 45 days. When the newest generic fact is the low-confidence weighted
fallback, a point-in-time fact can remain preferred for up to 185 days.
Duration selection is form-aware and deterministic.

Built-in reviewed logic covers established equal-economic and conversion cases
such as Alphabet, Meta, Mastercard, Nike, Palantir, Visa, and Berkshire
Hathaway. The declarative
[`data/sec_share_policies.json`](../data/sec_share_policies.json) registry
covers current ambiguous multi-class, tracking-stock, Up-C, partnership,
separately traded class, and SPAC structures. It records the displayed ticker,
confidence, price basis, policy rationale, official filing, exact member
multipliers, and any exact additional filing-fact selectors. The current
audited build resolves all 1,880 candidates, including all 900 sector top-100
positions; that is a property of this catalog revision, not a promise that
future filings will remain compatible.

The builder fails closed when an expected member is missing, renamed,
duplicated inconsistently, joined by an unreviewed class, placed under an
unknown dimension, or no longer matches an accession-scoped additional fact.
A changed filing structure invalidates the issuer estimate instead of silently
falling back to an older reviewed filing. Policies are calculation metadata,
not extra sector tiles. A provider-supplied cap remains authoritative because
separately traded classes can have different prices and some policies
deliberately use the canonical ticker as a documented provider-style proxy.

`EntityPublicFloat` is a filer-reported issuer-level value and is **not market
capitalization**. The build stores it as a numeric size proxy with provenance;
it never writes it into `Company.market_cap`. Runtime first accepts a valid cap
supplied with the current provider snapshot. Otherwise, when both a catalog
share estimate and a current snapshot price exist, a corporate-action-capable
provider supplies intervening forward and reverse splits. Runtime applies their
ratios to the dated price-equivalent common shares before multiplying by current
price. A failed required split lookup leaves the local estimate unavailable
instead of combining stale shares with a post-split price. Each successful
candidate snapshot refresh then selects 100 companies per sector using
estimated market cap when available or numeric public float otherwise. Those
900 companies and the three explicitly configured benchmark ETF proxies
receive the bulk daily and all-provider-available weekly history backfills.

This means a company can move into the visible top 100 as prices change if it
is already in the resolved candidate pool and has usable shares. A large
proxy-only issuer no longer loses automatically to every issuer with any known
cap. A new issuer, newly eligible filer, or company outside that pool requires
a successful catalog publication, but no longer requires a client release.
Public-float and share facts can have different as-of dates, and a missing
share fact leaves membership dependent on the proxy.

The selected asset provider's response refreshes names and exchange identifiers
for symbols it recognizes without overwriting SEC-derived SIC sector, numeric
proxy, share estimate/provenance, or retention state. Provider-only active
symbols remain searchable and can load detail, but do not enter a sector
without catalog metadata.

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
- `EntityPublicFloat` and share facts use issuer-specific filing practices;
  screening catches only obvious anomalies.
- A low-confidence weighted-average count is not a point-in-time count.
  Price-equivalent multi-class estimates depend on reviewed conversion,
  exchange, economic-equivalence, or provider-convention assumptions. The
  Alpaca adapter reconciles intervening forward and reverse splits, but
  amendments and other corporate actions can still make an estimate stale.
- A new or changed multi-class issuer, ADR, foreign filer, or CIK
  reorganization can remain unresolved until reviewed. Publication fails when
  such a gap reaches a current sector top-100 position; lower-ranked gaps are
  reported in the audit metadata.
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
  --user-agent "stock-tui catalog support@chatcode.dev" \
  --through 2026Q2 \
  --artifact-output build/catalog/sec-catalog.json.gz
python3 -m unittest discover -s tools/tests
cargo test universe
```

The tool accepts `SEC_USER_AGENT` instead of `--user-agent`, caches source
downloads under `~/.cache/stock-tui/sec-catalog` by default, restricts requests
to `www.sec.gov`, `data.sec.gov`, and `xbrl.sec.gov`, and defaults to eight
requests per second. It refuses a setting above the SEC's current aggregate
maximum of ten requests per second, retries transient failures, writes source
receipts, validates unique CIKs/symbols, consecutive per-sector ranks, safe SIC
labels, exact reviewed filing signatures, absolute share-fact freshness, and
complete top-100 share coverage, then atomically replaces the output.

`.github/workflows/catalog-publish.yml` runs daily at 06:17 UTC and can be
dispatched manually. It restores the large SEC source cache, invalidates
mutable ticker, Frames, and submissions responses, runs the network-free
calculation fixtures, builds and validates the complete audit catalog, packages
the compact deterministic artifact, and publishes it with Wrangler. Immutable versioned
objects are written before the five-minute-cache stable object and manifest.
The full audit catalog is retained only as a short-lived private workflow
artifact; scheduled builds do not commit generated catalogs to the repository.

The R2 bucket is `stock-tui-catalog`, with `stock.chatcode.dev` attached as its
custom domain. `infra/cloudflare/` contains the idempotent provisioning,
cache-invalidation, and publication scripts. CI requires a truthful
`SEC_USER_AGENT`, a Cloudflare account ID, and a bucket-scoped R2 write token as
GitHub secrets. It never receives market-provider credentials.

Reviewed releases replace their embedded fallback with one R2 catalog
download before the cross-platform build matrix, ensuring every platform
binary for that release contains identical catalog data. Local source builds
retain the reviewed repository snapshot.

Manual changes to selection logic must still review source dates, sector
counts, large rank movements, share methods/confidence, class policies, and
quality labels before merge. The automated job executes the reviewed model; it
does not invent new multi-class policy. Reusing immutable SEC downloads
improves efficiency, while mutable inputs are deliberately refreshed.

The builder follows the SEC's
[Developer Resources and Fair Access guidance](https://www.sec.gov/about/developer-resources).
Runtime catalog refreshes read R2 or the local fallback and do not poll SEC.

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
public-float proxy and share estimates can become stale between catalog
releases. A valid provider snapshot cap takes precedence. Otherwise, when
shares are available and the adapter can establish split coverage from their
as-of date, the client applies intervening forward and reverse split ratios and
estimates market cap as adjusted price-equivalent common shares times the
canonical ticker price. If that required lookup fails, the local estimate is
cleared rather than knowingly mixing pre-split shares with a post-split price.
The target is price-equivalent common equity, not fully diluted value.
Reviewed Up-C policies can include outstanding exchangeable operating units,
but preferred shares, RSUs, options, and unexercised convertibles are excluded.
Low-confidence policies identify weighted-average, separately traded-class, or
provider-style total-common approximations rather than presenting them as
exact economic equivalence.

Market-cap ordering compares the estimated cap where present and the numeric
public-float proxy otherwise, then uses catalog rank and symbol for stable
ties. A proxy-only ticker still shows market cap as unavailable. Gain and
volume ordering follow the selected range. Heatmap volume prefers the provider
snapshot's latest-session cumulative volume for `1D` and uses a cached OHLCV
sum for longer ranges; ticker-detail statistics retain that latest-session
snapshot volume. Alphabetical ordering uses ticker symbol.

This remains an approximation. Different traded classes can have different
prices; canonical-price proxy policies, treasury-share treatment, ADR ratios,
amendments, filing lag, delayed corporate-action publication, and actions other
than forward/reverse splits can all cause divergence from a commercial
fundamentals provider. Low-confidence policies identify the most approximate
cases. No Yahoo page, cookie, or unofficial scraped feed is used at build or
runtime.

## Adding A Provider

A provider contribution must include:

1. Official provenance and API documentation.
2. Written analysis of personal display, caching, retention, attribution, and
   redistribution rights.
3. Exact feed coverage, delay, corporate-action adjustment, market calendar,
   IANA timezone, currency, regular-session, and symbol-namespace semantics.
4. Secret-safe configuration and redacted errors.
5. Pagination, timeout, retry, and rate-limit behavior.
6. Fixture-based tests that never call a paid or credentialed service.
7. A mapping strategy that leaves unknown sectors and instruments explicit.

At the code boundary, an adapter implements `AssetProvider` and
`MarketDataProvider`; `CorporateActionsProvider` and `NewsProvider` are
optional and may be supplied independently. A provider-supplied snapshot cap
therefore remains the portable enrichment path when an adapter cannot supply
split history. An older local share estimate is used without split coverage
only when its as-of date matches the snapshot date. The capabilities are
assembled into `ProviderSet`, selected through `--provider`/configuration, and
must keep provider payloads and errors out of the domain, storage, and rendering
layers. A new provider also needs its own credential/onboarding branch where
applicable.

Every assembled provider must set two persistence properties:

- A stable cache namespace that changes whenever the endpoint, feed, adjustment
  policy, upstream dataset, or normalized contract can change observations.
- A `MarketContext` containing the calendar ID, symbol namespace, currency,
  IANA timezone, and regular-session bounds used to group chart observations.

The runtime permits one such market context per launch and validates it before
rendering cached rows. An incompatible or legacy unstamped cache is cleared
transactionally while favorite symbols survive for rehydration. Do not derive
the context from each company's free-text listing exchange: NASDAQ, NYSE, and
ARCA all belong to the current `us-equities` context because they share the
same chart calendar. A provider serving unrelated markets must expose separate
contexts (and currently requires separate launches/configurations) rather than
putting colliding symbols or sessions into one cache.

`StockApiProvider` is the reference for an adapter that does not reuse Alpaca
credentials and can operate either anonymously or with its own bearer token. It
validates a versioned normalized wire schema and omits `NewsProvider` entirely
when news is disabled. It does not change the licensing review required for a
concrete service deployment.

Do not add scraping of a website that forbids automated access or data reuse.
An API being technically reachable does not make its data redistributable.
