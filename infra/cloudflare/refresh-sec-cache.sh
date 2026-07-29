#!/usr/bin/env bash
set -euo pipefail

cache_dir="${1:?usage: refresh-sec-cache.sh CACHE_DIR}"
if [[ -z "$cache_dir" || "$cache_dir" == "/" ]]; then
  printf 'refusing unsafe cache directory: %q\n' "$cache_dir" >&2
  exit 1
fi

mkdir -p "$cache_dir"
removed=0
shopt -s nullglob
for metadata in "$cache_dir"/*.meta.json; do
  url="$(jq -er '.url // empty' "$metadata")"
  case "$url" in
    https://www.sec.gov/files/company_tickers_exchange.json|https://data.sec.gov/api/xbrl/frames/*|https://data.sec.gov/submissions/CIK*.json)
      stem="${metadata%.meta.json}"
      rm -f -- "$metadata" "$stem".*
      removed=$((removed + 1))
      ;;
  esac
done

printf 'invalidated %d mutable SEC cache entries\n' "$removed"
