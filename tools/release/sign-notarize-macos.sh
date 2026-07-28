#!/usr/bin/env bash
set -Eeuo pipefail

usage() {
  cat <<'EOF'
Usage: sign-notarize-macos.sh <binary> <payload-directory> <output.dmg> <volume-name>

Signs a macOS command-line binary with Developer ID and the hardened runtime,
creates a signed disk image, submits it to Apple's notary service, and staples
and validates the resulting ticket.
EOF
}

require_environment() {
  local name
  for name in \
    MACOS_SIGNING_CERT_P12_BASE64 \
    MACOS_SIGNING_CERT_PASSWORD \
    APPLE_NOTARY_API_KEY_P8_BASE64 \
    APPLE_NOTARY_KEY_ID \
    APPLE_NOTARY_ISSUER_ID
  do
    if [[ -z "${!name:-}" ]]; then
      printf 'Required release credential %s is not set.\n' "$name" >&2
      return 1
    fi
  done
}

if [[ "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ "$#" -ne 4 ]]; then
  usage >&2
  exit 64
fi

if [[ "$(uname -s)" != "Darwin" ]]; then
  printf 'macOS signing must run on macOS.\n' >&2
  exit 69
fi

binary_path="$1"
payload_directory="$2"
dmg_path="$3"
volume_name="$4"

if [[ ! -f "$binary_path" || ! -x "$binary_path" ]]; then
  printf 'Expected an executable binary at %s.\n' "$binary_path" >&2
  exit 66
fi

if [[ ! -d "$payload_directory" ]]; then
  printf 'Expected a payload directory at %s.\n' "$payload_directory" >&2
  exit 66
fi

case "$dmg_path" in
  *.dmg) ;;
  *)
    printf 'Disk image output must end in .dmg.\n' >&2
    exit 64
    ;;
esac

require_environment

for command in codesign hdiutil jq openssl plutil security spctl xcrun; do
  if ! command -v "$command" >/dev/null 2>&1; then
    printf 'Required macOS release tool %s is not available.\n' "$command" >&2
    exit 69
  fi
done
if [[ ! -x /usr/libexec/PlistBuddy ]]; then
  printf 'Required macOS release tool /usr/libexec/PlistBuddy is unavailable.\n' >&2
  exit 69
fi

umask 077
temporary_root="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
work_directory="$(mktemp -d "${temporary_root%/}/stock-tui-signing.XXXXXX")"
certificate_path="$work_directory/developer-id.p12"
notary_key_path="$work_directory/notary-api-key.p8"
notary_result_path="$work_directory/notary-submit.json"
notary_stderr_path="$work_directory/notary-submit.stderr"
notary_log_path="$work_directory/notary-log.json"
notary_log_stderr_path="$work_directory/notary-log.stderr"
entitlements_path="$work_directory/binary-entitlements.plist"
entitlements_stderr_path="$work_directory/binary-entitlements.stderr"
keychain_path="$work_directory/signing.keychain-db"
keychain_password="$(openssl rand -base64 32)"

cleanup() {
  security delete-keychain "$keychain_path" >/dev/null 2>&1 || true
  rm -rf "$work_directory"
}
trap cleanup EXIT HUP INT TERM

printf '%s' "$MACOS_SIGNING_CERT_P12_BASE64" \
  | /usr/bin/base64 -D >"$certificate_path"
printf '%s' "$APPLE_NOTARY_API_KEY_P8_BASE64" \
  | /usr/bin/base64 -D >"$notary_key_path"

security create-keychain -p "$keychain_password" "$keychain_path"
security set-keychain-settings -lut 21600 "$keychain_path"
security unlock-keychain -p "$keychain_password" "$keychain_path"
security import "$certificate_path" \
  -k "$keychain_path" \
  -P "$MACOS_SIGNING_CERT_PASSWORD" \
  -T /usr/bin/codesign \
  -T /usr/bin/security >/dev/null
security set-key-partition-list \
  -S apple-tool:,apple:,codesign: \
  -s \
  -k "$keychain_password" \
  "$keychain_path" >/dev/null

signing_identity="$(
  security find-identity -v -p codesigning "$keychain_path" \
    | awk '/Developer ID Application:/ { print $2; exit }'
)"
if [[ -z "$signing_identity" ]]; then
  printf 'The certificate does not contain a Developer ID Application identity.\n' >&2
  exit 65
fi

codesign \
  --force \
  --identifier com.chatcode-lab.stock-tui \
  --keychain "$keychain_path" \
  --options runtime \
  --sign "$signing_identity" \
  --timestamp \
  "$binary_path"
codesign --verify --strict --verbose=2 "$binary_path"

signature_details="$(codesign --display --verbose=4 "$binary_path" 2>&1)"
if ! grep -Eq 'flags=.*\(runtime\)' <<<"$signature_details"; then
  printf 'The binary signature does not enable the hardened runtime.\n' >&2
  exit 65
fi
if ! grep -q '^Timestamp=' <<<"$signature_details"; then
  printf 'The binary signature does not contain a secure timestamp.\n' >&2
  exit 65
fi

if ! codesign --display --entitlements :- "$binary_path" \
  >"$entitlements_path" \
  2>"$entitlements_stderr_path"
then
  printf 'Could not inspect the binary entitlements.\n' >&2
  exit 65
fi
if [[ -s "$entitlements_path" ]]; then
  if ! plutil -lint "$entitlements_path" >/dev/null; then
    printf 'The binary contains malformed entitlements.\n' >&2
    exit 65
  fi
  get_task_allow="$(
    /usr/libexec/PlistBuddy \
      -c 'Print :com.apple.security.get-task-allow' \
      "$entitlements_path" \
      2>/dev/null \
      || true
  )"
  case "$get_task_allow" in
    true | TRUE | True | 1)
      printf 'The release binary enables com.apple.security.get-task-allow.\n' >&2
      exit 65
      ;;
  esac
fi

mkdir -p "$(dirname "$dmg_path")"
rm -f "$dmg_path"
hdiutil create \
  -fs HFS+ \
  -format UDZO \
  -ov \
  -srcfolder "$payload_directory" \
  -volname "$volume_name" \
  "$dmg_path"

codesign \
  --force \
  --keychain "$keychain_path" \
  --sign "$signing_identity" \
  --timestamp \
  "$dmg_path"
codesign --verify --strict --verbose=2 "$dmg_path"
hdiutil verify "$dmg_path"

printf 'Submitting %s to Apple notarization.\n' "$dmg_path"
set +e
xcrun notarytool submit "$dmg_path" \
  --issuer "$APPLE_NOTARY_ISSUER_ID" \
  --key "$notary_key_path" \
  --key-id "$APPLE_NOTARY_KEY_ID" \
  --output-format json \
  --wait \
  >"$notary_result_path" \
  2>"$notary_stderr_path"
notary_exit="$?"
set -e

submission_id=""
notary_status=""
notary_response_path=""
if jq -e 'type == "object"' "$notary_result_path" >/dev/null 2>&1; then
  notary_response_path="$notary_result_path"
elif jq -e 'type == "object"' "$notary_stderr_path" >/dev/null 2>&1; then
  notary_response_path="$notary_stderr_path"
fi
if [[ -n "$notary_response_path" ]]; then
  submission_id="$(jq -r '.id // empty' "$notary_response_path")"
  notary_status="$(jq -r '.status // empty' "$notary_response_path")"
fi

if [[ -z "$submission_id" ]]; then
  printf 'Apple notarization did not return a submission ID (exit %s).\n' \
    "$notary_exit" >&2
  exit 65
fi

printf 'Apple notarization submission %s completed with status %s (exit %s).\n' \
  "$submission_id" \
  "${notary_status:-unknown}" \
  "$notary_exit"

set +e
xcrun notarytool log "$submission_id" \
  --issuer "$APPLE_NOTARY_ISSUER_ID" \
  --key "$notary_key_path" \
  --key-id "$APPLE_NOTARY_KEY_ID" \
  --output-format json \
  "$notary_log_path" \
  >/dev/null \
  2>"$notary_log_stderr_path"
notary_log_exit="$?"
set -e

if [[ "$notary_log_exit" -ne 0 ]] \
  || ! jq -e 'type == "object"' "$notary_log_path" >/dev/null 2>&1
then
  printf 'Could not retrieve a valid log for notarization submission %s.\n' \
    "$submission_id" >&2
  exit 65
fi

notary_log_status="$(jq -r '.status // empty' "$notary_log_path")"
notary_issue_count="$(jq -r '(.issues // []) | length' "$notary_log_path")"
notary_error_count="$(
  jq -r \
    '[ (.issues // [])[] | select(((.severity // "") | ascii_downcase) == "error") ] | length' \
    "$notary_log_path"
)"
printf 'Apple notarization log status is %s with %s issue(s), %s error(s).\n' \
  "${notary_log_status:-unknown}" \
  "$notary_issue_count" \
  "$notary_error_count"

if [[ "$notary_issue_count" -ne 0 ]]; then
  jq -r \
    '(.issues // [])[] | "[notary \(.severity // "unknown")] \(.message // "No message") (\(.path // "no path"))"' \
    "$notary_log_path"
fi

if [[ "$notary_exit" -ne 0 ]] \
  || [[ "$notary_status" != "Accepted" ]] \
  || [[ "$notary_log_status" != "Accepted" ]] \
  || [[ "$notary_error_count" -ne 0 ]]
then
  printf 'Apple did not accept notarization submission %s without errors.\n' \
    "$submission_id" >&2
  exit 65
fi

xcrun stapler staple -v "$dmg_path"
xcrun stapler validate -v "$dmg_path"
hdiutil verify "$dmg_path"
spctl \
  --assess \
  --context context:primary-signature \
  --type open \
  --verbose=2 \
  "$dmg_path"

printf 'Signed, notarized, and stapled %s.\n' "$dmg_path"
