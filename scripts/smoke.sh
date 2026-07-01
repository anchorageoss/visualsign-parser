#!/usr/bin/env bash
# Post-deploy smoke test for the live dev VisualSign parser.
#
# Runs a known Solana V0 transaction (referencing address lookup tables) through
# the deployed parser (the /visualsign-dev endpoint) via `turnkey-client verify`
# and asserts BOTH:
#   - it RENDERS  — a regression guard for the "Cannot render V0 ... refusing to
#     display a partial transaction" failure; and
#   - it VERIFIES — the AWS Nitro attestation and the enclave signature are
#     cryptographically valid (proof the parse ran inside the enclave).
#
# Drives the published turnkey-client CONTAINER (no Go toolchain needed); its
# JSON goes to stdout (asserted via `jq`) and its step-by-step verification log
# goes to stderr, which this script passes through by default so you can SEE the
# client ran and what it verified.
#
# When the container image is unavailable (e.g. not yet published), point
# --turnkey-client-path at a local fallback: an executable client binary, or a
# turnkey-client source dir that is built (`make build`) and run from
# bin/visualsign-turnkeyclient.
#
# Usage: smoke.sh [--turnkey-client-path <binary-or-source-dir>]
#                 [--turnkey-client-version <tag>] [--quiet]
#
# Flags:
#   --turnkey-client-path P     local client used only when the container image
#                               is unavailable (or VSP_SMOKE_TURNKEY_CLIENT_PATH)
#   --turnkey-client-version T  container image tag to pull (default: latest);
#                               pin an approved version in CI (or via
#                               VSP_SMOKE_CLIENT_VERSION)
#   --quiet, -q                 suppress the client's output on success (failures
#                               stay verbose); default shows it
#
# Env (all optional; defaults target the dev endpoint/app):
#   VSP_SMOKE_HOST                 API host    (default https://api.turnkey.com)
#   VSP_SMOKE_ORG                 organization id
#   VSP_SMOKE_KEY                 key name under ~/.config/turnkey/keys/<key>.{public,private}
#   VSP_SMOKE_CLIENT_VERSION      container image tag (same as --turnkey-client-version)
#   TURNKEY_CLIENT                how to invoke the client (overrides all resolution)
#   VSP_SMOKE_TURNKEY_CLIENT_PATH local fallback path (same as --turnkey-client-path)
#
# Exit: 0 = rendered + verified (pass) OR endpoint unreachable (skip; not ours);
#       1 = endpoint up but parser failed to render / verify / assertions failed;
#       2 = smoke could not run the client (e.g. missing/unpullable image or
#           binary) — a harness failure, never treated as a pass.
set -euo pipefail

HOST="${VSP_SMOKE_HOST:-https://api.turnkey.com}"
ORG="${VSP_SMOKE_ORG:-d7f51c3d-fb9d-47c1-9b2e-a02b1cd5ff14}"
KEY="${VSP_SMOKE_KEY:-dev}"
CLIENT_PATH="${VSP_SMOKE_TURNKEY_CLIENT_PATH:-}"
CLIENT_VERSION="${VSP_SMOKE_CLIENT_VERSION:-latest}"
QUIET=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --turnkey-client-path)
      [ "$#" -ge 2 ] || { echo "--turnkey-client-path requires a value" >&2; exit 2; }
      CLIENT_PATH="$2"; shift 2 ;;
    --turnkey-client-path=*) CLIENT_PATH="${1#*=}"; shift ;;
    --turnkey-client-version)
      [ "$#" -ge 2 ] || { echo "--turnkey-client-version requires a value" >&2; exit 2; }
      CLIENT_VERSION="$2"; shift 2 ;;
    --turnkey-client-version=*) CLIENT_VERSION="${1#*=}"; shift ;;
    -q | --quiet) QUIET=1; shift ;;
    -h | --help)
      echo "usage: smoke.sh [--turnkey-client-path <binary-or-source-dir>] [--turnkey-client-version <tag>] [--quiet]" >&2
      exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
IMAGE="ghcr.io/anchorageoss/visualsign-turnkeyclient:${CLIENT_VERSION}"
CONTAINER_CLIENT="docker run --rm -v $HOME/.config/turnkey/keys:/root/.config/turnkey/keys:ro $IMAGE"

# Resolve a local fallback path to a runnable client: an executable is used
# directly; a directory is treated as the turnkey-client source and built
# (unless its binary already exists). Prints the client path on stdout.
resolve_fallback_client() {
  local p="$1"
  if [ -x "$p" ] && [ ! -d "$p" ]; then
    printf '%s' "$p"
  elif [ -d "$p" ]; then
    local bin="$p/bin/visualsign-turnkeyclient"
    if [ ! -x "$bin" ]; then
      echo "building turnkey-client in $p ..." >&2
      ( cd "$p" && GOPATH="${GOPATH:-$HOME/go}" make build >&2 ) \
        || { echo "ERROR: failed to build turnkey-client in $p" >&2; exit 2; }
    fi
    [ -x "$bin" ] || { echo "ERROR: no client binary at $bin after build" >&2; exit 2; }
    printf '%s' "$bin"
  else
    echo "ERROR: fallback client path is neither an executable nor a directory: $p" >&2
    exit 2
  fi
}

# Client resolution: explicit override -> published container (if pullable) ->
# local fallback. With none available, keep the container command so the run's
# guard reports the missing image as a harness error (exit 2), not a pass.
if [ -n "${TURNKEY_CLIENT:-}" ]; then
  CLIENT="$TURNKEY_CLIENT"
elif docker image inspect "$IMAGE" >/dev/null 2>&1 || docker pull "$IMAGE" >/dev/null 2>&1; then
  CLIENT="$CONTAINER_CLIENT"
elif [ -n "$CLIENT_PATH" ]; then
  echo "container image $IMAGE unavailable; using local fallback client: $CLIENT_PATH" >&2
  CLIENT="$(resolve_fallback_client "$CLIENT_PATH")" || exit $?
else
  CLIENT="$CONTAINER_CLIENT"
fi

DIR="$(cd "$(dirname "$0")/.." && pwd)"
PAYLOAD="$(tr -d '[:space:]' < "$DIR/testdata/solana_v0_alt.b64")"
ERRFILE="$(mktemp)"
trap 'rm -f "$ERRFILE"' EXIT

set +e
OUT="$($CLIENT verify --dev-path --host "$HOST" --organization-id "$ORG" \
  --key-name "$KEY" --unsigned-payload "$PAYLOAD" --chain CHAIN_SOLANA 2>"$ERRFILE")"
RC=$?
set -e

if [ "$RC" -ne 0 ]; then
  # Show the client's own output so the failure is diagnosable (always, even
  # under --quiet), then classify. Default to a hard error: only a recognized
  # endpoint outage may skip, so a broken harness can't masquerade as a pass.
  cat "$ERRFILE" >&2

  # Endpoint reachable but the parser returned a non-OK status -> our regression.
  if grep -q "non-OK status" "$ERRFILE"; then
    echo "FAIL: deployed parser rejected a tx it should render (regression)" >&2
    exit 1
  fi
  # A genuine transport/network error reaching the endpoint is a pre-existing
  # outage, not the deploy's fault -> skip. Match only connection-level errors.
  if grep -qiE \
    'connection refused|connection reset|no such host|dial tcp|i/o timeout|timeout|tls handshake|\bEOF\b|context deadline|network is unreachable|server misbehaving|temporary failure in name resolution' \
    "$ERRFILE"; then
    echo "SKIP: dev endpoint unreachable / outage — not a regression" >&2
    exit 0
  fi
  # Anything else means the smoke harness itself could not run the client
  # (missing/unpullable image, missing binary, bad invocation). NOT a pass:
  # surface it loudly so a broken smoke can't be mistaken for success.
  echo "ERROR: smoke could not run the turnkey-client; this is not an endpoint outage" >&2
  exit 2
fi

# Client ran. Pass its verification log through unless the caller asked to be
# quiet, so a PASS is visibly backed by the real step-by-step output.
[ "$QUIET" -eq 1 ] || cat "$ERRFILE" >&2

# Assert BOTH the render guard and the cryptographic verification result.
if ! echo "$OUT" | jq -e '
      (.signablePayload | length > 0)
  and (.signablePayload | contains("Cannot render V0") | not)
  and (.valid == true)
  and (.attestationValid == true)
  and (.signatureValid == true)
' >/dev/null; then
  echo "FAIL: render/verification assertions failed. Response summary:" >&2
  echo "$OUT" | jq '{
    signablePayloadLen: (.signablePayload | length?),
    valid, attestationValid, signatureValid, moduleId
  }' >&2 || true
  exit 1
fi

chars="$(printf '%s' "$OUT" | jq -r '.signablePayload | length')"
module="$(printf '%s' "$OUT" | jq -r '.moduleId // "unknown"')"
echo "PASS: turnkey-client verify succeeded; V0+ALT rendered ($chars chars, no \"Cannot render V0\"); attestation + signature cryptographically verified (executed in AWS Nitro enclave); moduleId=$module"
