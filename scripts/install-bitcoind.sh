#!/usr/bin/env bash
# Install the pinned Bitcoin Core release (bitcoind + bitcoin-cli) for the
# regtest validation lane.
#
# The version and sha256 live in .github/bitcoind.env. This script downloads the
# exact release tarball from bitcoincore.org and refuses it unless the sha256
# matches.
#
# Usage: scripts/install-bitcoind.sh [install-dir]
#   install-dir defaults to $RUNNER_TEMP/bitcoind (or ./bitcoind outside Actions).
#   Prints the bin dir; on GitHub Actions it is also appended to $GITHUB_PATH.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
config="$repo_root/.github/bitcoind.env"

config_value() {
  local value
  value="$(grep -E "^$1=" "$config" | cut -d= -f2-)"
  if [[ -z "$value" ]]; then
    echo "install-bitcoind: $1 is not set in $config" >&2
    exit 1
  fi
  echo "$value"
}

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) platform=x86_64-linux-gnu ;;
  *)
    echo "install-bitcoind: no pinned Bitcoin Core for $(uname -s)-$(uname -m)" >&2
    exit 1
    ;;
esac

version="$(config_value BITCOIND_VERSION)"
expected="$(config_value "BITCOIND_SHA256_$platform")"
install_dir="${1:-${RUNNER_TEMP:-$PWD}/bitcoind}"
tarball="$install_dir/bitcoin-$version-$platform.tar.gz"

mkdir -p "$install_dir"
curl --fail --silent --show-error --location --retry 3 --output "$tarball" \
  "https://bitcoincore.org/bin/bitcoin-core-$version/bitcoin-$version-$platform.tar.gz"
actual="$(sha256sum "$tarball" | cut -d' ' -f1)"
if [[ "$actual" != "$expected" ]]; then
  echo "install-bitcoind: sha256 mismatch: expected $expected, got $actual" >&2
  exit 1
fi
tar -xzf "$tarball" -C "$install_dir"
rm "$tarball"

bin="$install_dir/bitcoin-$version/bin"
"$bin/bitcoind" -version | head -1
if [[ -n "${GITHUB_PATH:-}" ]]; then
  echo "$bin" >> "$GITHUB_PATH"
fi
echo "$bin"
