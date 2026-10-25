#!/usr/bin/env bash
# Install the pinned protoc release and put it on PATH.
#
# The version and per-platform sha256 live in .github/protoc.env. This script
# downloads the exact release asset and checks its hash, instead of
# arduino/setup-protoc, which resolves even a pinned version by paging through
# the GitHub releases API on every run (and fails CI when that lookup does).
#
# Usage:
#   scripts/install-protoc.sh [install-dir]
#     Install into install-dir (default $RUNNER_TEMP/protoc, or ./protoc outside
#     Actions). On GitHub Actions the bin dir is appended to $GITHUB_PATH.
#   scripts/install-protoc.sh --update <version>
#     Download every platform asset for <version>, hash it, and rewrite
#     .github/protoc.env.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
config="$repo_root/.github/protoc.env"
platforms=(linux-x86_64 linux-aarch_64 osx-aarch_64 osx-x86_64)

sha256_of() {
  if command -v sha256sum >/dev/null; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

asset_url() {
  echo "https://github.com/protocolbuffers/protobuf/releases/download/v$1/protoc-$1-$2.zip"
}

download() {
  curl --fail --silent --show-error --location --retry 3 --output "$2" "$1"
}

# Reads KEY's value from the config without sourcing it.
config_value() {
  local value
  value="$(grep -E "^$1=" "$config" | cut -d= -f2-)"
  if [[ -z "$value" ]]; then
    echo "install-protoc: $1 is not set in $config" >&2
    exit 1
  fi
  echo "$value"
}

if [[ "${1:-}" == "--update" ]]; then
  version="${2:?usage: $0 --update <version>}"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  {
    echo "# The protoc release CI installs, and its sha256 per platform asset."
    echo "# Read by scripts/install-protoc.sh. Bump with: scripts/install-protoc.sh --update <version>"
    echo "PROTOC_VERSION=$version"
    for platform in "${platforms[@]}"; do
      download "$(asset_url "$version" "$platform")" "$tmp/$platform.zip"
      echo "PROTOC_SHA256_$platform=$(sha256_of "$tmp/$platform.zip")"
    done
  } > "$tmp/protoc.env"
  mv "$tmp/protoc.env" "$config"
  echo "Updated $config to protoc $version"
  exit 0
fi

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) platform=linux-x86_64 ;;
  Linux-aarch64 | Linux-arm64) platform=linux-aarch_64 ;;
  Darwin-arm64) platform=osx-aarch_64 ;;
  Darwin-x86_64) platform=osx-x86_64 ;;
  *)
    echo "install-protoc: no pinned protoc for $(uname -s)-$(uname -m)" >&2
    exit 1
    ;;
esac

version="$(config_value PROTOC_VERSION)"
expected="$(config_value "PROTOC_SHA256_$platform")"
install_dir="${1:-${RUNNER_TEMP:-$PWD}/protoc}"
zip="$install_dir/protoc-$version-$platform.zip"

mkdir -p "$install_dir"
download "$(asset_url "$version" "$platform")" "$zip"
actual="$(sha256_of "$zip")"
if [[ "$actual" != "$expected" ]]; then
  echo "install-protoc: sha256 mismatch for $(basename "$zip"): expected $expected, got $actual" >&2
  exit 1
fi
unzip -o -q "$zip" -d "$install_dir"
rm "$zip"

"$install_dir/bin/protoc" --version

if [[ -n "${GITHUB_PATH:-}" ]]; then
  echo "$install_dir/bin" >> "$GITHUB_PATH"
fi
