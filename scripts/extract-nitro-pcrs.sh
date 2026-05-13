#!/usr/bin/env bash
# Build the tkhq/qos enclave image at the rev pinned in src/Cargo.toml and
# extract /nitro.pcrs from the resulting OCI image.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"

CARGO_TOML="$REPO_ROOT/src/Cargo.toml"
OUTPUT="$REPO_ROOT/out/nitro.pcrs"
SKOPEO_IMAGE="quay.io/skopeo/stable:latest"
QOS_DIR=""
REV=""

QOS_DIR_AUTO=0
STAGE_DIR=""
CID=""

usage() {
  cat <<EOF
Usage: $(basename "$0") [options]

Build the tkhq/qos enclave image at the rev pinned in src/Cargo.toml and
extract /nitro.pcrs from the resulting OCI image.

Options:
  --cargo-toml PATH    Workspace Cargo.toml          (default: $CARGO_TOML)
  --qos-dir PATH       Where to clone/reuse qos      (default: mktemp -d, removed on exit)
  --output PATH        Where to write nitro.pcrs     (default: $OUTPUT)
  --skopeo-image REF   Pinned skopeo image           (default: $SKOPEO_IMAGE)
  --rev REV            Override rev (skip Cargo.toml)
  -h, --help           Show this help and exit
EOF
}

die() {
  echo "$*" >&2
  exit 1
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --cargo-toml)   CARGO_TOML="$2"; shift 2 ;;
      --qos-dir)      QOS_DIR="$2"; shift 2 ;;
      --output)       OUTPUT="$2"; shift 2 ;;
      --skopeo-image) SKOPEO_IMAGE="$2"; shift 2 ;;
      --rev)          REV="$2"; shift 2 ;;
      -h|--help)      usage; exit 0 ;;
      *) die "Unknown argument: $1" ;;
    esac
  done
}

read_rev() {
  [[ -f "$CARGO_TOML" ]] || die "Cargo.toml not found: $CARGO_TOML"
  local revs
  mapfile -t revs < <(
    grep -oE 'git = "https://github.com/tkhq/qos\.git", rev = "[0-9a-f]{40}"' "$CARGO_TOML" \
      | grep -oE '[0-9a-f]{40}' \
      | sort -u
  )
  [[ ${#revs[@]} -ge 1 ]] || die "No qos git deps found in $CARGO_TOML"
  [[ ${#revs[@]} -eq 1 ]] || die "qos revs disagree in $CARGO_TOML: ${revs[*]}"
  REV="${revs[0]}"
}

ensure_qos_checkout() {
  if [[ -z "$QOS_DIR" ]]; then
    QOS_DIR="$(mktemp -d -t visualsign-qos.XXXXXX)"
    QOS_DIR_AUTO=1
  fi

  if [[ -d "$QOS_DIR/.git" ]]; then
    local head
    head="$(git -C "$QOS_DIR" rev-parse HEAD)"
    if [[ "$head" == "$REV" ]]; then
      echo "Reusing qos checkout at $QOS_DIR (HEAD=$head)" >&2
      return
    fi
    [[ -z "$(git -C "$QOS_DIR" status --porcelain)" ]] \
      || die "qos checkout $QOS_DIR has uncommitted changes"
    echo "Updating qos checkout in $QOS_DIR to $REV" >&2
    git -C "$QOS_DIR" fetch --quiet origin "$REV"
    git -C "$QOS_DIR" checkout --quiet "$REV"
    return
  fi

  echo "Cloning tkhq/qos into $QOS_DIR" >&2
  git clone --quiet https://github.com/tkhq/qos.git "$QOS_DIR"
  git -C "$QOS_DIR" checkout --quiet "$REV"
}

build_qos_enclave() {
  echo "Building qos_enclave at rev $REV (may take several minutes)..." >&2
  make -C "$QOS_DIR" out/qos_enclave/index.json
}

extract_pcrs() {
  local oci_dir="$QOS_DIR/out/qos_enclave"
  [[ -f "$oci_dir/index.json" ]] || die "qos build did not produce $oci_dir/index.json"

  STAGE_DIR="$(mktemp -d -t visualsign-pcrs.XXXXXX)"

  docker run --rm \
    -v "$oci_dir:/src:ro" \
    -v "$STAGE_DIR:/dst" \
    "$SKOPEO_IMAGE" \
    copy oci:/src:latest "docker-archive:/dst/qos_enclave.tar:qos-enclave:latest"

  docker load -i "$STAGE_DIR/qos_enclave.tar"
  CID="$(docker create qos-enclave:latest)"

  mkdir -p "$(dirname "$OUTPUT")"
  docker cp "$CID:/nitro.pcrs" "$OUTPUT"
  [[ -s "$OUTPUT" ]] || die "nitro.pcrs not found in qos-enclave image"
}

cleanup() {
  if [[ -n "$CID" ]]; then
    docker rm "$CID" >/dev/null 2>&1 || true
  fi
  if [[ -n "$STAGE_DIR" ]]; then
    rm -rf "$STAGE_DIR"
  fi
  if [[ "$QOS_DIR_AUTO" -eq 1 && -n "$QOS_DIR" ]]; then
    rm -rf "$QOS_DIR"
  fi
}

main() {
  parse_args "$@"
  if [[ -z "$REV" ]]; then
    read_rev
  fi
  trap cleanup EXIT
  ensure_qos_checkout
  build_qos_enclave
  extract_pcrs
  echo "Wrote $OUTPUT:" >&2
  cat "$OUTPUT"
  echo
}

main "$@"
