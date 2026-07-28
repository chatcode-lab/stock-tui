#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
config="${WRANGLER_CONFIG:-$repo_root/infra/cloudflare/wrangler.jsonc}"
wrangler_version="${WRANGLER_VERSION:-4.114.0}"
domain="${R2_PUBLIC_DOMAIN:-stock.chatcode.dev}"
bucket="$(jq -er '.r2_buckets[] | select(.binding == "CATALOG_BUCKET") | .bucket_name' "$config")"

prompt_for_value() {
  local name="$1"
  local prompt="$2"
  local secret="${3:-0}"
  local value="${!name:-}"
  if [[ -n "$value" ]]; then
    return
  fi
  if [[ ! -t 0 ]]; then
    printf '%s\n' \
      "$name is required for non-interactive provisioning." \
      "Set it in the process environment; do not run wrangler login." >&2
    exit 1
  fi
  if [[ "$secret" == "1" ]]; then
    read -rsp "$prompt" value
    printf '\n'
  else
    read -rp "$prompt" value
  fi
  [[ -n "$value" ]] || {
    printf '%s must not be empty.\n' "$name" >&2
    exit 1
  }
  printf -v "$name" '%s' "$value"
  export "$name"
}

prompt_for_value CLOUDFLARE_API_TOKEN "Cloudflare API token: " 1
prompt_for_value CLOUDFLARE_ACCOUNT_ID "Cloudflare account ID: "
prompt_for_value CLOUDFLARE_ZONE_ID "chatcode.dev zone ID: "

for id_name in CLOUDFLARE_ACCOUNT_ID CLOUDFLARE_ZONE_ID; do
  [[ "${!id_name}" =~ ^[A-Fa-f0-9]{32}$ ]] || {
    printf '%s must be a 32-character Cloudflare ID.\n' "$id_name" >&2
    exit 1
  }
done

export CI="${CI:-1}"
wrangler=(npx --yes "wrangler@$wrangler_version")
if ! "${wrangler[@]}" r2 bucket list --config "$config" >/dev/null; then
  printf '%s\n' \
    "Cloudflare API-token authentication failed." \
    "Use a Cloudflare custom API token with Workers R2 Storage: Edit." >&2
  exit 1
fi
printf 'Cloudflare API-token authentication succeeded.\n'

if "${wrangler[@]}" r2 bucket info "$bucket" --json --config "$config" >/dev/null 2>&1; then
  printf 'R2 bucket already exists: %s\n' "$bucket"
else
  printf 'R2 bucket is absent or inaccessible; attempting creation: %s\n' "$bucket"
  "${wrangler[@]}" r2 bucket create "$bucket" --config "$config"
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
