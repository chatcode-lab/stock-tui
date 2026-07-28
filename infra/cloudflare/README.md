# SEC Catalog Publishing

This directory publishes only the compact SEC-derived issuer catalog. It does
not receive, cache, or redistribute market-provider prices, bars, volume, news,
or credentials.

## Public Objects

The `stock-tui-catalog` R2 bucket is exposed through the custom domain
`stock.chatcode.dev`:

| Object | Purpose | Cache policy |
| --- | --- | --- |
| `/catalog/sec-catalog.json` | Stable compact catalog, transported with gzip content encoding | 5 minutes, with one-hour stale reuse |
| `/catalog/sec-catalog.manifest.json` | Stable artifact checksum and metadata | 5 minutes, with one-hour stale reuse |
| `/catalog/versions/<version>/<timestamp>-<sha>.json` | Immutable compact catalog | 1 year, immutable |
| Matching `.manifest.json` | Immutable checksum and metadata | 1 year, immutable |

The object named `.json` contains deterministic gzip bytes. R2 metadata sets
`Content-Type: application/json` and `Content-Encoding: gzip`, so normal HTTP
clients receive JSON after transparent decompression.

## One-Time Provisioning

Prerequisites are Node/npm, `jq`, Wrangler 4.114.0, access to the Cloudflare
account containing the `chatcode.dev` zone, and its zone ID.

```bash
npx --yes wrangler@4.114.0 login
npx --yes wrangler@4.114.0 whoami
CLOUDFLARE_ZONE_ID=<chatcode-dev-zone-id> \
  infra/cloudflare/provision-catalog-r2.sh
```

The provisioning script idempotently creates `stock-tui-catalog` and attaches
`stock.chatcode.dev` with TLS 1.2 or newer. Keep the account's `r2.dev`
development URL disabled.

Cloudflare does not cache JSON by default. Add a zone Cache Rule matching:

```text
Hostname equals stock.chatcode.dev
AND URI Path starts with /catalog/
```

Mark matching responses eligible for cache and respect the origin
`Cache-Control` header. Do not override the immutable and stable object
lifetimes set by the publisher.

## Initial Publication

Build a fresh audit catalog with a truthful SEC contact identity, package it,
and run the same validated publisher used by CI:

```bash
export SEC_USER_AGENT="stock-tui catalog <maintainer-contact>"
mkdir -p build/catalog
python3 tools/build_sec_catalog.py \
  --output build/catalog/sec_universe.json \
  --artifact-output build/catalog/sec-catalog.json.gz \
  --artifact-manifest-output build/catalog/sec-catalog.manifest.json
infra/cloudflare/publish-catalog.sh \
  build/catalog/sec_universe.json \
  build/catalog/sec-catalog.json.gz \
  build/catalog/sec-catalog.manifest.json
```

Use `CATALOG_PUBLISH_DRY_RUN=1` to execute every local validation without
uploading.

## Scheduled Publication

`.github/workflows/catalog-publish.yml` runs daily at 06:17 UTC and supports
manual dispatch. Scheduled runs remain skipped until the repository variable
`CATALOG_PUBLISH_ENABLED` is set to `true`. Configure these GitHub Actions
repository secrets:

| Secret | Required scope |
| --- | --- |
| `CLOUDFLARE_ACCOUNT_ID` | The account containing `stock-tui-catalog` |
| `CLOUDFLARE_API_TOKEN` | R2 object read/write for this account or, preferably, this bucket only |
| `SEC_USER_AGENT` | Truthful application and maintainer contact required by SEC fair-access policy |

The recurring token does not need DNS, zone, Worker script, D1, or general
account administration permission. Provision the bucket and custom domain
separately with Wrangler OAuth or a one-time administrative token.

After the first publication has been verified, enable scheduled runs:

```bash
gh variable set CATALOG_PUBLISH_ENABLED \
  --repo chatcode-lab/stock-tui \
  --body true
```

The workflow cache stores large immutable SEC downloads. Before each build,
`refresh-sec-cache.sh` invalidates ticker associations and XBRL Frames inputs
that can change. The full audit JSON is retained as a private Actions artifact
for 14 days; only the compact artifact and manifest are sent to R2.

## Validation

Run the local checks without Cloudflare credentials:

```bash
python3 -m unittest discover -s tools/tests
python3 tools/build_sec_catalog.py \
  --package-only \
  --output data/sec_universe.json \
  --artifact-output /tmp/sec-catalog.json.gz \
  --artifact-manifest-output /tmp/sec-catalog.manifest.json
CATALOG_PUBLISH_DRY_RUN=1 \
  infra/cloudflare/publish-catalog.sh \
  data/sec_universe.json \
  /tmp/sec-catalog.json.gz \
  /tmp/sec-catalog.manifest.json
```
