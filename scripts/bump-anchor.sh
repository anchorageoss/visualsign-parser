#!/usr/bin/env bash
# Bump the pinned Anchor release the anchor-cli image installs.
#
# The version, its sha256 and the upstream repo live in .github/anchor.env;
# images/anchor-cli/Containerfile's ARG defaults must carry the same values,
# or a plain `docker build` (no --build-arg) would install a different release
# than the one recorded in the env file.
#
# Usage:
#   scripts/bump-anchor.sh
#     Move both pins to the newest Anchor release in ANCHOR_SERIES (e.g. 0.32)
#     that is at least 7 days old and ships an x86_64-unknown-linux-gnu asset whose sha256 the GitHub API
#     records as the asset digest. The asset is downloaded and its hash is
#     checked against that digest before anything is rewritten. Prints
#     "unchanged" or "bumped A -> B".
#   scripts/bump-anchor.sh --check
#     Fail if anchor.env and the Containerfile ARG defaults disagree.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
env_file="$repo_root/.github/anchor.env"
containerfile="$repo_root/images/anchor-cli/Containerfile"
min_age_days=7
asset_suffix="x86_64-unknown-linux-gnu"

sha256_of() {
  sha256sum "$1" | cut -d' ' -f1
}

# Reads KEY's value from anchor.env without sourcing it.
env_value() {
  local value
  value="$(grep -E "^$1=" "$env_file" | cut -d= -f2-)"
  if [[ -z "$value" ]]; then
    echo "bump-anchor: $1 is not set in $env_file" >&2
    exit 1
  fi
  printf '%s\n' "$value"
}

# Reads an ARG default from the Containerfile.
arg_default() {
  sed -n "s/^ARG $1=//p" "$containerfile"
}

check() {
  local key env_val arg_val failed=0
  for key in ANCHOR_VERSION ANCHOR_SHA256 ANCHOR_REPO; do
    env_val="$(env_value "$key")"
    arg_val="$(arg_default "$key")"
    if [[ "$env_val" != "$arg_val" ]]; then
      echo "bump-anchor: $key disagrees: $env_file has '$env_val', $containerfile has '$arg_val'" >&2
      failed=1
    fi
  done
  if ((failed)); then
    exit 1
  fi
  echo "ok: anchor.env and the Containerfile ARG defaults agree"
}

# Releases in ANCHOR_SERIES, highest version first, that are not drafts or
# prereleases, are at least 7 days old,
# and whose x86_64-unknown-linux-gnu asset carries a sha256 digest, as
# "version<TAB>sha256" (newest first).
candidates() {
  local repo series cutoff
  repo="$(env_value ANCHOR_REPO)"
  series="$(env_value ANCHOR_SERIES)"
  cutoff="$(date -u -d "$min_age_days days ago" +%s)"
  gh api --paginate "repos/$repo/releases?per_page=100" |
    jq -sr --argjson cutoff "$cutoff" --arg suffix "$asset_suffix" --arg series "$series" '
      add
      | map(select(.tag_name | startswith("v" + $series + ".")))
      | sort_by(.tag_name | ltrimstr("v") | split(".") | map(tonumber? // 0))
      | reverse | .[]
      | select((.draft // false) | not)
      | select(.prerelease | not)
      | select((.published_at | sub("\\.[0-9]+Z$"; "Z") | fromdateiso8601) <= $cutoff)
      | (.tag_name | ltrimstr("v")) as $version
      | ([.assets[]?
          | select(.name == "anchor-" + $version + "-" + $suffix)]
         | first) as $asset
      | select($asset != null)
      | select($asset.digest != null and ($asset.digest | startswith("sha256:")))
      | [$version, ($asset.digest | ltrimstr("sha256:"))]
      | @tsv
    '
}

bump() {
  local repo cur_version cur_sha version digest sha tmp
  repo="$(env_value ANCHOR_REPO)"
  cur_version="$(env_value ANCHOR_VERSION)"
  cur_sha="$(env_value ANCHOR_SHA256)"

  version=""
  # Newest first, so the first eligible line is the release to pin.
  while IFS=$'\t' read -r version digest; do
    break
  done < <(candidates)
  if [[ -z "$version" ]]; then
    echo "bump-anchor: no non-prerelease Anchor $(env_value ANCHOR_SERIES).x release at least $min_age_days days old with an $asset_suffix asset digest" >&2
    exit 1
  fi
  if [[ "$version" == "$cur_version" && "$digest" == "$cur_sha" ]]; then
    echo "unchanged"
    return 0
  fi

  # Download the exact URL the Containerfile will fetch and check it against
  # the digest the API recorded, so a rewritten pin is always verified.
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  url="https://github.com/$repo/releases/download/v$version/anchor-$version-$asset_suffix"
  curl --fail --silent --show-error --location --retry 3 --output "$tmp/anchor" "$url"
  sha="$(sha256_of "$tmp/anchor")"
  if [[ "$sha" != "$digest" ]]; then
    echo "bump-anchor: sha256 mismatch for anchor $version: API digest $digest, download hashed $sha" >&2
    exit 1
  fi

  sed -i "s/^ANCHOR_VERSION=.*/ANCHOR_VERSION=$version/" "$env_file"
  sed -i "s/^ANCHOR_SHA256=.*/ANCHOR_SHA256=$sha/" "$env_file"
  sed -i "s/^ARG ANCHOR_VERSION=.*/ARG ANCHOR_VERSION=$version/" "$containerfile"
  sed -i "s/^ARG ANCHOR_SHA256=.*/ARG ANCHOR_SHA256=$sha/" "$containerfile"
  echo "bumped $cur_version -> $version"
}

case "${1:-}" in
  --check) check ;;
  "") bump ;;
  *) echo "usage: $0 [--check]" >&2
     exit 2 ;;
esac
