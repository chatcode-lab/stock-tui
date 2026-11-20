#!/usr/bin/env bash
set -euo pipefail

cache_dir="${1:?usage: refresh-sec-cache.sh CACHE_DIR}"
if [[ -z "$cache_dir" || "$cache_dir" == "/" ]]; then
  printf 'refusing unsafe cache directory: %q\n' "$cache_dir" >&2
  exit 1
fi

mkdir -p "$cache_dir"
removed=0
year="$(date -u +%Y)"
month="$(date -u +%m)"
quarter=$(( (10#$month - 1) / 3 + 1 ))
previous_year="$year"
previous_quarter=$((quarter - 1))
if [[ "$previous_quarter" -eq 0 ]]; then
  previous_year=$((year - 1))
  previous_quarter=4
fi

current_index="https://www.sec.gov/Archives/edgar/full-index/$year/QTR$quarter/master.zip"
previous_index="https://www.sec.gov/Archives/edgar/full-index/$previous_year/QTR$previous_quarter/master.zip"
shopt -s nullglob
for metadata in "$cache_dir"/*.meta.json; do
  url="$(jq -er '.url // empty' "$metadata")"
  case "$url" in
    https://www.sec.gov/files/company_tickers_exchange.json|https://data.sec.gov/api/xbrl/frames/*|https://data.sec.gov/submissions/CIK*.json|"$current_index"|"$previous_index")
      stem="${metadata%.meta.json}"
      rm -f -- "$metadata" "$stem".*
      removed=$((removed + 1))
      ;;
  esac
done

printf 'invalidated %d mutable SEC cache entries\n' "$removed"
