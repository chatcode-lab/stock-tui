# Stock API HTTP Contract

The `stock-api` adapter lets `stock-tui` consume normalized market data from a
separately operated HTTP service. The contract is intentionally independent of
any upstream vendor. Implementing the HTTP shape does not grant the operator
rights to collect, cache, display, or redistribute any underlying data.

The client does not send Alpaca credentials, cookies, or generic API-key
headers to this service. It optionally sends a service-specific bearer token
from `STOCK_TUI_STOCK_API_TOKEN` or the lower-precedence
`providers.stock_api.token` TOML setting; when unset, no authorization header
is sent.

## Base URL And Versioning

Select the adapter and service explicitly:

```bash
stock-tui \
  --provider stock-api \
  --stock-api-url http://127.0.0.1:8787
```

The configured base URL excludes the version prefix. The client preserves its
path and appends `v1/...`:

| Base URL | Assets endpoint |
| --- | --- |
| `https://example.com` | `https://example.com/v1/assets` |
| `https://example.com/api` | `https://example.com/api/v1/assets` |

HTTPS is required. Plain HTTP is accepted only for `localhost`, `127.0.0.1`,
and `::1`, which supports controlled local Worker tests. User information,
queries, and fragments are rejected in base URLs.

Every successful response is a JSON object:

```json
{
  "schema_version": 1,
  "data": [],
  "next_page_token": null
}
```

`schema_version` must equal `1`. `data` is always an array.
`next_page_token` is either an opaque non-empty string or `null`; it is used
only by assets and bars. Unknown response fields may be added compatibly, but
removing or changing documented fields requires a new route version.

All timestamps are RFC 3339 instants. JSON numbers must be finite. Symbols are
ASCII, case-insensitive on input, at most 32 characters, and limited to
letters, digits, `.` and `-`; the service should return canonical uppercase
symbols. The v1 runtime namespace uses dotted share-class and preferred-series
suffixes such as `BF.A`, `BRK.B`, and `TRTN.PRA`. A catalog-backed service must
convert SEC hyphen notation such as `BF-A` and `TRTN-PA` before validation,
lookup, and persistence, and reject symbols that collide after conversion.

## Market Context

The current `stock-api` v1 client assigns this adapter the `us-equities` market
context: US equity symbols, USD, `America/New_York`, and a regular session from
09:30 through 16:00 local exchange time. A v1 service used with this client
must return observations compatible with that single context. The free-text
asset `exchange` field is listing metadata; it does not create separate caches
for NASDAQ, NYSE, or ARCA.

The configured base URL and contract version are part of the client's cache
namespace. Changing the base URL therefore clears unattributable provider rows
before the new service is used. A future service that exposes non-US markets,
another currency, an independent symbol namespace, or different session rules
needs a versioned context-negotiation extension and corresponding client
support. It must not place those observations into the current v1 US context.

## Request Headers

The client sends:

```http
Accept: application/json
User-Agent: stock-tui/<version>
```

When either supported token source is configured, the client additionally
sends:

```http
Authorization: Bearer <token>
```

There is deliberately no CLI token flag. The token and even its presence are
excluded from `--print-config`, debug output, and logs. It is used only by
`StockApiProvider`, never by Alpaca or another adapter. Redirects are disabled
so a configured endpoint cannot forward the header to another URL. When both
sources are unset, the header is absent and unauthenticated compatible services
continue to work. Keep a token-bearing `.env` or `config.toml` private.
Tokens are limited to 4,096 ASCII token68 bytes after trimming surrounding
whitespace: alphanumerics and `-._~+/`, followed by optional `=` padding.
Responses should use `Content-Type: application/json`.

## Active Assets

```http
GET /v1/assets?status=active
GET /v1/assets?status=active&page_token=<opaque>
```

Asset item:

```json
{
  "symbol": "AAPL",
  "name": "Apple Inc.",
  "exchange": "NASDAQ",
  "market_cap": 3200000000000.0,
  "updated_at": "2026-07-28T15:59:00Z"
}
```

`market_cap` and `updated_at` are optional. A supplied market cap must be
positive and finite. When `updated_at` is absent, the client uses receipt time.
The client keeps its SEC-derived sector, share estimate, size proxy, rank, and
provenance. A valid asset market cap is retained when the local catalog has no
share estimate. A valid value on the subsequent snapshot always takes
precedence over a local `shares x price` approximation.

Assets support opaque pagination. The client rejects repeated tokens, more
than 100 pages, more than 100,000 accumulated assets, or a response body above
32 MiB.

## Snapshots

```http
GET /v1/snapshots?symbols=AAPL,MSFT
```

Snapshot item:

```json
{
  "symbol": "AAPL",
  "price": 213.5,
  "market_cap": 3200000000000.0,
  "market_cap_estimate": {
    "value": 3200000000000.0,
    "currency": "USD",
    "price_as_of": "2026-07-28T16:00:00Z",
    "shares_as_of": "2026-06-30",
    "calculated_at": "2026-07-28T16:00:01Z",
    "method": "snapshot_price_x_sec_price_equivalent_shares",
    "confidence": "high"
  },
  "previous_close": 210.0,
  "open": 211.0,
  "high": 214.0,
  "low": 209.5,
  "volume": 12345.0,
  "as_of": "2026-07-28T16:00:00Z"
}
```

All numeric observation fields are nullable. Prices and OHLC values must be
positive when present; volume must be non-negative. `high` cannot be below
`low`. `as_of` is required. The response must not include unrequested symbols
or a pagination token.

`market_cap` is optional, positive, and finite. The client uses a valid supplied
value immediately and gives it precedence over a local `shares x price`
approximation. The `stock-api` adapter does not currently expose a separate
corporate-actions capability, so absent a provider cap the client uses a local
share estimate only when its as-of date matches the snapshot date; an older
count remains unavailable rather than combining pre-split shares with a
post-split price. `market_cap_estimate` is optional service provenance and is
currently ignored by the client, but compatible services should preserve the
shown value, currency, price/share dates, calculation time, method, and
confidence fields rather than implying that an estimate is a
provider-reported fundamental.

The client sends no more than 100 symbols per snapshot request.

## Adjusted Bars

```http
GET /v1/bars?symbols=AAPL,MSFT&timeframe=1Day&start=2026-07-01T00%3A00%3A00Z&end=2026-07-29T00%3A00%3A00Z&adjustment=all
GET /v1/bars?...&page_token=<opaque>
```

Supported client timeframe values are `5Min`, `1Hour`, `1Day`, and `1Week`.
`start` is inclusive and `end` is exclusive. The client requires `end` to be
later than `start`. `adjustment=all` means the returned OHLCV series must apply
the service's documented full corporate-action adjustment policy; a service
that cannot provide that semantic should reject the request rather than return
an unlabelled incompatible series.

Bar item:

```json
{
  "symbol": "AAPL",
  "timeframe": "1Day",
  "timestamp": "2026-07-28T20:00:00Z",
  "open": 213.0,
  "high": 215.0,
  "low": 212.0,
  "close": 214.0,
  "volume": 11000.0,
  "trade_count": 42,
  "vwap": 213.8,
  "source": "licensed-feed"
}
```

OHLC values must be positive and internally consistent with the high/low
range. Volume must be non-negative. `trade_count` and `vwap` are nullable.
`source` is a required, provider-neutral provenance label stored with the bar;
the service must supply any attribution required by its data rights. The
response timeframe and symbols must match the request.

A service may preserve a no-trade placeholder as a flat bar with zero volume
and zero or absent trade count. The client retains such a row in raw SQLite but
does not treat it as a price observation, freshness update, history boundary,
or chart point.

The client sends no more than 50 symbols per bars request. Bars support opaque
pagination with the same token/page/body protections as assets and a maximum
of 1,000,000 accumulated bars per request.

## Optional News

Enable or omit the news capability with `--stock-api-news`,
`STOCK_TUI_STOCK_API_NEWS`, or `[providers.stock_api].news` in `config.toml`.
When disabled, the client does not register a `NewsProvider` and never requests
this route.

```http
GET /v1/news?symbols=AAPL,MSFT&limit=20
```

News item:

```json
{
  "id": "article-123",
  "headline": "Company publishes results",
  "source": "Publisher",
  "published_at": "2026-07-28T18:00:00Z",
  "url": "https://publisher.example/story",
  "summary": "Concise optional summary.",
  "symbols": ["AAPL"]
}
```

`summary` and `symbols` may be omitted and default to an empty value. `id`,
`headline`, `source`, `published_at`, and an HTTP(S) `url` are required. The
client caps `limit` at 50, sorts newest first, and rejects pagination for this
endpoint.

## Errors, Limits, And Caching

Errors use an HTTP status plus a safe JSON body:

```json
{
  "error": {
    "code": "not_entitled",
    "message": "Requested data is unavailable"
  }
}
```

`code` is a stable machine identifier. `message` is safe for logs and terminal
display and must contain no secrets or upstream credentials. The client maps
`401`, `403`, and `429` into authentication, permission, and rate-limit error
classes and preserves other status codes generically.

Each request has a 20-second timeout. The client retries timeouts, `408`, `429`,
`500`, `502`, `503`, and `504` at most three times after the initial request.
It honors numeric or HTTP-date `Retry-After` values up to 30 seconds and uses
bounded exponential backoff otherwise. Each decoded response body is limited
to 32 MiB.

The service may return `ETag`, `Last-Modified`, `Age`, and `Cache-Control`.
The current client does not persist validators or issue conditional requests;
SQLite is its durable observation cache. A CDN may cache by the complete URL,
including symbols, range, timeframe, adjustment, limit, and page token.
Operators must choose TTLs, stale behavior, and purge rules that comply with
their data licenses and the time sensitivity of each endpoint.

## Deployment Status

`https://stock.chatcode.dev/api` is the project-operated private development
endpoint and default base URL. It requires a bearer token distributed out of
band to authorized testers and is not a licensed public market-data service.
Its downstream limiter allows 120 requests per rolling 60-second window per
SHA-256 token fingerprint. Responses are cached by complete request identity;
the service returns `429` plus `Retry-After` after the limit is exhausted.

For local Cloudflare Worker development without downstream authentication, use:

```bash
stock-tui \
  --provider stock-api \
  --stock-api-url http://127.0.0.1:8787
```

The service operator is responsible for upstream authorization, attribution,
delay labels, extraction controls, retention, geography, and redistribution
rights. The client contract neither removes nor obscures those obligations.
