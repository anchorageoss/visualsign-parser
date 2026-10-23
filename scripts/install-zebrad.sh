#!/usr/bin/env bash
# Install the pinned Zebra release (zebrad) and the Sapling proving parameters
# for the Zcash regtest validation lane.
#
# Versions and sha256s live in .github/zebrad.env. This script downloads the
# exact release tarball and parameter files and refuses any whose sha256 does
# not match. Files already present with the right sha256 are kept.
#
# Usage: scripts/install-zebrad.sh [install-dir]
#   install-dir defaults to $RUNNER_TEMP/zebrad (or ./zebrad outside Actions).
#   Prints the install dir (zebrad and the params are directly in it); on
#   GitHub Actions it is also appended to $GITHUB_PATH.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
config="$repo_root/.github/zebrad.env"

config_value() {
  local value
  value="$(grep -E "^$1=" "$config" | cut -d= -f2-)"
  if [[ -z "$value" ]]; then
    echo "install-zebrad: $1 is not set in $config" >&2
    exit 1
  fi
  echo "$value"
}

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) platform=x86_64-unknown-linux-gnu ;;
  *)
    echo "install-zebrad: no pinned zebrad for $(uname -s)-$(uname -m)" >&2
    exit 1
    ;;
esac

install_dir="${1:-${RUNNER_TEMP:-$PWD}/zebrad}"
mkdir -p "$install_dir"

# fetch <url> <dest> <sha256>
fetch() {
  if [[ -f "$2" ]] && [[ "$(sha256sum "$2" | cut -d' ' -f1)" == "$3" ]]; then
    return
  fi
  curl --fail --silent --show-error --location --retry 3 --output "$2.part" "$1"
  local actual
  actual="$(sha256sum "$2.part" | cut -d' ' -f1)"
  if [[ "$actual" != "$3" ]]; then
    echo "install-zebrad: sha256 mismatch for $1: expected $3, got $actual" >&2
    rm -f "$2.part"
    exit 1
  fi
  mv "$2.part" "$2"
}

version="$(config_value ZEBRAD_VERSION)"
tarball="$install_dir/zebrad-$version-$platform.tar.gz"
fetch "https://github.com/ZcashFoundation/zebra/releases/download/v$version/zebrad-$version-$platform.tar.gz" \
  "$tarball" "$(config_value "ZEBRAD_SHA256_$platform")"
tar -xzf "$tarball" -C "$install_dir" zebrad
rm "$tarball"

for name in spend output; do
  upper="$(echo "$name" | tr '[:lower:]' '[:upper:]')"
  fetch "https://download.z.cash/downloads/sapling-$name.params" \
    "$install_dir/sapling-$name.params" "$(config_value "SAPLING_${upper}_SHA256")"
done

"$install_dir/zebrad" --version | head -1 >&2
if [[ -n "${GITHUB_PATH:-}" ]]; then
  echo "$install_dir" >> "$GITHUB_PATH"
fi
echo "$install_dir"
