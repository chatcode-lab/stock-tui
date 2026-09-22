#!/usr/bin/env bash
set -euo pipefail

if (( $# != 1 )); then
  printf 'Usage: %s <cargo-package-file-list>\n' "$0" >&2
  exit 2
fi

file_list=$1
if [[ ! -f "$file_list" ]]; then
  printf 'Cargo package file list not found: %s\n' "$file_list" >&2
  exit 1
fi

forbidden=()
while IFS= read -r path; do
  [[ -n "$path" ]] || continue
  case "$path" in
    .env.example) continue ;;
    .env | .env.* | .env-* | */.env | */.env.* | */.env-* \
      | credentials.env | */credentials.env \
      | config.toml | */config.toml \
      | *.p8 | *.p12 | *.pem | *.key \
      | *.db | *.db-shm | *.db-wal | *.db-journal \
      | *.sqlite | *.sqlite-shm | *.sqlite-wal | *.sqlite-journal \
      | *.sqlite3 | *.sqlite3-shm | *.sqlite3-wal | *.sqlite3-journal \
      | .stock-tui/* | */.stock-tui/* \
      | .wrangler/* | */.wrangler/* \
      | logs-*.json | */logs-*.json)
      forbidden+=("$path")
      ;;
  esac
done < "$file_list"

if (( ${#forbidden[@]} > 0 )); then
  printf 'Refusing to package credential or local runtime data paths:\n' >&2
  printf '  %s\n' "${forbidden[@]}" >&2
  exit 1
fi
