#!/usr/bin/env bash
# Install a pinned protoc release for CI and put it on PATH.
#
# Downloads the exact release asset and checks its sha256, instead of
# arduino/setup-protoc, which resolves even a pinned version by paging through
# the GitHub releases API on every run (and fails CI when that lookup does).
#
# Usage: scripts/install-protoc.sh [install-dir]
#   install-dir defaults to $RUNNER_TEMP/protoc (or ./.protoc outside Actions).
#   On GitHub Actions the bin dir is appended to $GITHUB_PATH.
set -euo pipefail

PROTOC_VERSION="21.4"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)
    asset="protoc-${PROTOC_VERSION}-linux-x86_64.zip"
    sha256="d51e8f030162f08823a4738ab0ac00bee537e30b583a562e6962dbb040d86736"
    ;;
  *)
    echo "install-protoc: no pinned protoc ${PROTOC_VERSION} for $(uname -s)-$(uname -m)" >&2
    exit 1
    ;;
esac

install_dir="${1:-${RUNNER_TEMP:-$PWD}/protoc}"
url="https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/${asset}"

mkdir -p "$install_dir"
curl --fail --silent --show-error --location --retry 3 --output "$install_dir/$asset" "$url"
echo "${sha256}  $install_dir/$asset" | sha256sum --check --strict
unzip -o -q "$install_dir/$asset" -d "$install_dir"
rm "$install_dir/$asset"

"$install_dir/bin/protoc" --version

if [[ -n "${GITHUB_PATH:-}" ]]; then
  echo "$install_dir/bin" >> "$GITHUB_PATH"
fi
