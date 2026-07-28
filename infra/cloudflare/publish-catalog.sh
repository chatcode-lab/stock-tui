#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
config="${WRANGLER_CONFIG:-$repo_root/infra/cloudflare/wrangler.jsonc}"
source_catalog="${1:?usage: publish-catalog.sh AUDIT_JSON ARTIFACT_JSON_GZ ARTIFACT_MANIFEST}"
artifact_path="${2:?usage: publish-catalog.sh AUDIT_JSON ARTIFACT_JSON_GZ ARTIFACT_MANIFEST}"
manifest_path="${3:?usage: publish-catalog.sh AUDIT_JSON ARTIFACT_JSON_GZ ARTIFACT_MANIFEST}"
wrangler_version="${WRANGLER_VERSION:-4.114.0}"
prefix="${R2_PREFIX:-catalog}"
public_base_url="${R2_PUBLIC_BASE_URL:-https://stock.chatcode.dev}"
dry_run="${CATALOG_PUBLISH_DRY_RUN:-0}"

for command in gzip jq npx sha256sum; do
  command -v "$command" >/dev/null || {
    printf 'required command is unavailable: %s\n' "$command" >&2
    exit 1
  }
done
for required_file in "$source_catalog" "$artifact_path" "$manifest_path"; do
  [[ -f "$required_file" ]] || {
    printf 'catalog publication input does not exist: %s\n' "$required_file" >&2
    exit 1
  }
done
[[ -f "$config" ]] || {
  printf 'Wrangler config does not exist: %s\n' "$config" >&2
  exit 1
}

bucket="${R2_BUCKET:-$(jq -er '.r2_buckets[] | select(.binding == "CATALOG_BUCKET") | .bucket_name' "$config")}"
prefix="${prefix#/}"
prefix="${prefix%/}"
public_base_url="${public_base_url%/}"
[[ "$bucket" =~ ^[a-z0-9][a-z0-9-]{1,61}[a-z0-9]$ ]] || {
  printf 'invalid R2 bucket name: %s\n' "$bucket" >&2
  exit 1
}
[[ "$prefix" =~ ^[A-Za-z0-9._/-]+$ ]] || {
  printf 'invalid R2 object prefix: %s\n' "$prefix" >&2
  exit 1
}

jq -e '
  (.schema_version == 2)
  and (.catalog_version | type == "string" and length > 0)
  and (.generated_at | type == "string" and test("Z$"))
  and (.companies | type == "array" and length >= 900)
  and ([.companies[].symbol] | length == (unique | length))
  and ([.companies[].cik] | length == (unique | length))
  and ([.companies[].sector] | unique | length == 9)
  and all(.companies[];
    (.public_float | type == "number" and isfinite and . > 0)
    and (.proxy_source | type == "string" and length > 0)
    and (.proxy_as_of | type == "string" and length > 0)
    and (.proxy_confidence == "low")
  )
' "$source_catalog" >/dev/null

catalog_version="$(jq -er '.catalog_version' "$source_catalog")"
generated_at="$(jq -er '.generated_at' "$source_catalog")"
catalog_schema="$(jq -er '.schema_version' "$source_catalog")"
manifest_catalog_version="$(jq -er '.catalog.catalog_version' "$manifest_path")"
manifest_generated_at="$(jq -er '.catalog.generated_at' "$manifest_path")"
manifest_schema="$(jq -er '.catalog.schema_version' "$manifest_path")"
manifest_company_count="$(jq -er '.catalog.company_count' "$manifest_path")"
manifest_sha256="$(jq -er '.artifact.sha256' "$manifest_path")"
manifest_size="$(jq -er '.artifact.size_bytes' "$manifest_path")"
manifest_payload_sha256="$(jq -er '.artifact.payload_sha256' "$manifest_path")"
manifest_payload_size="$(jq -er '.artifact.payload_size_bytes' "$manifest_path")"

[[ "$manifest_catalog_version" == "$catalog_version" ]]
[[ "$manifest_generated_at" == "$generated_at" ]]
[[ "$manifest_schema" == "$catalog_schema" ]]
[[ "$(jq -r '.manifest_version' "$manifest_path")" == "1" ]]
[[ "$(jq -r '.artifact.compression' "$manifest_path")" == "gzip" ]]
[[ "$(jq -r '.artifact.content_type' "$manifest_path")" == "application/json" ]]
[[ "$(jq -r '.artifact.content_encoding' "$manifest_path")" == "gzip" ]]
gzip -t "$artifact_path"
actual_sha256="$(sha256sum "$artifact_path" | awk '{print $1}')"
actual_size="$(wc -c < "$artifact_path" | tr -d '[:space:]')"
actual_payload_sha256="$(gzip -cd "$artifact_path" | sha256sum | awk '{print $1}')"
actual_payload_size="$(gzip -cd "$artifact_path" | wc -c | tr -d '[:space:]')"
[[ "$actual_sha256" == "$manifest_sha256" ]]
[[ "$actual_size" == "$manifest_size" ]]
[[ "$actual_payload_sha256" == "$manifest_payload_sha256" ]]
[[ "$actual_payload_size" == "$manifest_payload_size" ]]
gzip -cd "$artifact_path" | jq -e \
  --argjson expected_count "$manifest_company_count" \
  '.schema_version == 2 and (.companies | length == $expected_count)' >/dev/null

version_slug="$(printf '%s' "$catalog_version" | tr -c 'A-Za-z0-9._-' '-')"
timestamp_slug="$(printf '%s' "$generated_at" | tr -d ':-')"
version_key="$prefix/versions/$version_slug/${timestamp_slug}-${actual_sha256}.json"
version_manifest_key="${version_key%.json}.manifest.json"
stable_key="$prefix/sec-catalog.json"
stable_manifest_key="$prefix/sec-catalog.manifest.json"

put_object() {
  local key="$1"
  local file="$2"
  local content_type="$3"
  local cache_control="$4"
  local content_encoding="${5:-}"
  if [[ "$dry_run" == "1" ]]; then
    printf 'would upload %s/%s (%s; %s; encoding=%s)\n' \
      "$bucket" "$key" "$content_type" "$cache_control" "${content_encoding:-identity}"
    return
  fi
  local -a arguments=(
    r2 object put "$bucket/$key"
    --file "$file"
    --content-type "$content_type"
    --cache-control "$cache_control"
    --remote
    --force
    --config "$config"
  )
  if [[ -n "$content_encoding" ]]; then
    arguments+=(--content-encoding "$content_encoding")
  fi
  npx --yes "wrangler@$wrangler_version" "${arguments[@]}"
}

immutable_cache='public, max-age=31536000, immutable'
current_cache='public, max-age=300, stale-while-revalidate=3600'

# Publish immutable compressed content first. The stable manifest is the final readiness marker.
put_object "$version_key" "$artifact_path" 'application/json' "$immutable_cache" 'gzip'
put_object "$version_manifest_key" "$manifest_path" 'application/json; charset=utf-8' "$immutable_cache"
put_object "$stable_key" "$artifact_path" 'application/json' "$current_cache" 'gzip'
put_object "$stable_manifest_key" "$manifest_path" 'application/json; charset=utf-8' "$current_cache"

printf 'catalog publication prepared: %s\n' "$public_base_url/$stable_key"
