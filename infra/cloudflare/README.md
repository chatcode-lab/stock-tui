# Issuer Catalog Publishing

This directory publishes only the compact SEC-derived issuer catalog and its
CC0 Wikidata company-context enrichment. It does not receive, cache, or
redistribute market-provider prices, bars, volume, news, or credentials.

## Public Objects

The `stock-tui-catalog` R2 bucket is exposed through the custom domain
`stock.chatcode.dev`:

| Object | Purpose | Cache policy |
| --- | --- | --- |
| `/catalog/sec-catalog.json` | Stable compact catalog, transported with gzip content encoding | 5 minutes, with one-hour stale reuse |
| `/catalog/sec-catalog.manifest.json` | Stable artifact checksum and metadata | 5 minutes, with one-hour stale reuse |
| `/catalog/wikidata-company-profiles-v1.json` | Durable schema-v2 builder snapshot of positive and empty per-CIK profile lookups and selected structured facts | Must revalidate |
| `/catalog/versions/<version>/<timestamp>-<sha>.json` | Immutable compact catalog | 1 year, immutable |
| Matching `.manifest.json` | Immutable checksum and metadata | 1 year, immutable |
| `/catalog/profile-versions/v2-a2/<sha>.json` | Immutable company-profile snapshot | 1 year, immutable |

The `sec-catalog.json` object contains deterministic gzip bytes. R2 metadata
sets `Content-Type: application/json` and `Content-Encoding: gzip`, so normal
HTTP clients receive JSON after transparent decompression. Profile snapshots
are ordinary UTF-8 JSON used only by the catalog builder.

## One-Time Provisioning

Prerequisites are Node/npm, `jq`, access to the Cloudflare account containing
the `chatcode.dev` zone, and its account and zone IDs.

Do not use `wrangler login` from a remote terminal. Its OAuth callback listens
on the remote machine's `localhost`, and browser bot challenges can prevent the
authorization page from returning JSON. Instead, open
<https://dash.cloudflare.com/profile/api-tokens> in your normal browser and
create a temporary **custom API token** with:

- Account / Workers R2 Storage / Edit for the intended account.
- Zone / Zone / Read for the `chatcode.dev` zone.

This is a Cloudflare REST API token, not an R2 S3 Access Key ID and Secret
Access Key pair. Wrangler reads it from `CLOUDFLARE_API_TOKEN`.

Run the provisioning script directly:

```bash
infra/cloudflare/provision-catalog-r2.sh
```

It securely prompts for the token without echoing it, then asks for the
account and zone IDs. The values exist only in the script process and are not
written to the repository, Wrangler OAuth storage, or shell history. For
non-interactive automation, export `CLOUDFLARE_API_TOKEN`,
`CLOUDFLARE_ACCOUNT_ID`, and `CLOUDFLARE_ZONE_ID` before invoking the script.

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

Build a fresh audit catalog with a truthful application/contact identity,
package it, and run the same validated publisher used by CI. The builder uses
that descriptive User-Agent for both SEC and Wikimedia requests:

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
  build/catalog/sec-catalog.manifest.json \
  ~/.cache/stock-tui/sec-catalog/wikidata-company-profiles-v1.json
```

Use `CATALOG_PUBLISH_DRY_RUN=1` to execute every local validation without
uploading.

## Scheduled Publication

`.github/workflows/catalog-publish.yml` runs daily at 06:17 UTC and supports
manual dispatch. Ordinary runs reuse the durable per-CIK company-profile
snapshot and fetch only previously unseen or materially renamed issuers. The
first scheduled run of each month refreshes every current profile; a manual
run can do the same by selecting `refresh_company_profiles`. Scheduled runs
remain skipped until the repository variable `CATALOG_PUBLISH_ENABLED` is set
to `true`. Configure these GitHub Actions repository secrets:

| Secret | Required scope |
| --- | --- |
| `CLOUDFLARE_ACCOUNT_ID` | The account containing `stock-tui-catalog` |
| `CLOUDFLARE_API_TOKEN` | Cloudflare REST token with Workers R2 Storage Write for the publishing account |
| `SEC_USER_AGENT` | Truthful application and maintainer contact required by SEC fair-access policy and reused for identified Wikimedia requests |

The recurring token does not need DNS, zone, Worker script, D1, or general
account administration permission. Provision the bucket and custom domain
separately with Wrangler OAuth or a one-time administrative token.

After the first publication has been verified, enable scheduled runs:

```bash
gh variable set CATALOG_PUBLISH_ENABLED \
  --repo chatcode-lab/stock-tui \
  --body true
```

The workflow cache stores large immutable SEC downloads and the versioned
per-CIK company-profile snapshot. Before each build, the workflow restores the
stable profile snapshot from R2, then `refresh-sec-cache.sh` invalidates only
mutable SEC ticker associations, XBRL Frames inputs, and submissions. Wikidata
positive and negative lookups remain stable between monthly refreshes, while
new CIKs are fetched on the next daily build. A profile-algorithm change
re-evaluates every current issuer. The schema-v2 working snapshot retains the
selected item, industries, products/services, and retrieval receipts under the
backward-compatible `wikidata-company-profiles-v1.json` object name. The full
audit JSON and profile snapshot are retained as private Actions artifacts for
14 days. R2 stores the compact catalog and manifest plus stable and
content-addressed profile snapshots.

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
