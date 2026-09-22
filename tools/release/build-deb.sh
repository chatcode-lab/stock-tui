#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf 'Usage: %s <binary> <version> <architecture> [output-directory]\n' "$0" >&2
}

if (( $# < 3 || $# > 4 )); then
  usage
  exit 2
fi

binary=$1
version=$2
architecture=$3
output_directory=${4:-.}

if [[ ! -x "$binary" ]]; then
  printf 'Executable not found: %s\n' "$binary" >&2
  exit 1
fi
if [[ ! "$version" =~ ^[0-9][0-9A-Za-z.+:~\-]*$ ]]; then
  printf 'Invalid Debian package version: %s\n' "$version" >&2
  exit 1
fi
case "$architecture" in
  amd64 | arm64) ;;
  *)
    printf 'Unsupported Debian architecture: %s\n' "$architecture" >&2
    exit 1
    ;;
esac
if ! command -v dpkg-deb >/dev/null 2>&1; then
  printf 'dpkg-deb is required to build a Debian package.\n' >&2
  exit 1
fi

repository_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
binary=$(cd "$(dirname "$binary")" && pwd)/$(basename "$binary")
mkdir -p "$output_directory"
output_directory=$(cd "$output_directory" && pwd)

work_directory=$(mktemp -d "${TMPDIR:-/tmp}/stock-tui-deb.XXXXXX")
trap 'rm -rf "$work_directory"' EXIT HUP INT TERM

package_root="$work_directory/package"
documentation="$package_root/usr/share/doc/stock-tui"
mkdir -p "$package_root/DEBIAN" "$package_root/usr/bin" "$documentation/examples"

install -m 0755 "$binary" "$package_root/usr/bin/stock-tui"
install -m 0644 "$repository_root/README.md" "$documentation/README.md"
install -m 0644 "$repository_root/LICENSE" "$documentation/copyright"
install -m 0644 "$repository_root/config.example.toml" \
  "$documentation/examples/config.toml"
install -m 0644 "$repository_root/.env.example" \
  "$documentation/examples/environment"
gzip -9 -n -c "$repository_root/CHANGELOG.md" > "$documentation/changelog.gz"
chmod 0644 "$documentation/changelog.gz"

installed_size=$(du -sk "$package_root/usr" | cut -f1)
cat > "$package_root/DEBIAN/control" <<EOF
Package: stock-tui
Version: $version
Architecture: $architecture
Maintainer: Chatcode Labs contributors <opensource@chatcode.dev>
Installed-Size: $installed_size
Section: utils
Priority: optional
Homepage: https://github.com/chatcode-lab/stock-tui
Description: mouse-first terminal stock market heatmap
 stock-tui presents sector heatmaps, detailed price and volume charts,
 company news, favorites, and locally cached market data in a responsive TUI.
EOF
chmod 0644 "$package_root/DEBIAN/control"

(
  cd "$package_root"
  find usr -type f -print0 | LC_ALL=C sort -z | xargs -0 md5sum \
    > DEBIAN/md5sums
)
chmod 0644 "$package_root/DEBIAN/md5sums"

if [[ -n "${SOURCE_DATE_EPOCH:-}" ]]; then
  if [[ ! "$SOURCE_DATE_EPOCH" =~ ^[0-9]+$ ]]; then
    printf 'SOURCE_DATE_EPOCH must be an integer.\n' >&2
    exit 1
  fi
  find "$package_root" -print0 \
    | xargs -0 touch --no-dereference --date="@$SOURCE_DATE_EPOCH"
fi

artifact="$output_directory/stock-tui_${version}_${architecture}.deb"
dpkg-deb --root-owner-group --build "$package_root" "$artifact"
printf 'Built %s\n' "$artifact"
