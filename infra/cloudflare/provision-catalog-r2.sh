#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
config="${WRANGLER_CONFIG:-$repo_root/infra/cloudflare/wrangler.jsonc}"
wrangler_version="${WRANGLER_VERSION:-4.114.0}"
domain="${R2_PUBLIC_DOMAIN:-stock.chatcode.dev}"
bucket="$(jq -er '.r2_buckets[] | select(.binding == "CATALOG_BUCKET") | .bucket_name' "$config")"

wrangler=(npx --yes "wrangler@$wrangler_version")
if "${wrangler[@]}" r2 bucket info "$bucket" --json --config "$config" >/dev/null 2>&1; then
  printf 'R2 bucket already exists: %s\n' "$bucket"
else
  printf 'R2 bucket is absent or inaccessible; attempting creation: %s\n' "$bucket"
  "${wrangler[@]}" r2 bucket create "$bucket" --config "$config"
fi

if [[ -z "${CLOUDFLARE_ZONE_ID:-}" ]]; then
  printf '%s\n' \
    "Bucket is ready, but the custom domain was not attached." \
    "Set CLOUDFLARE_ZONE_ID for the chatcode.dev zone and rerun this script."
  exit 0
fi

if "${wrangler[@]}" r2 bucket domain get "$bucket" \
  --domain "$domain" \
  --config "$config" >/dev/null 2>&1; then
  printf 'R2 custom domain already exists: %s\n' "$domain"
else
  "${wrangler[@]}" r2 bucket domain add "$bucket" \
    --domain "$domain" \
    --zone-id "$CLOUDFLARE_ZONE_ID" \
    --min-tls 1.2 \
    --force \
    --config "$config"
fi

printf 'R2 catalog base URL: https://%s/catalog/\n' "$domain"
