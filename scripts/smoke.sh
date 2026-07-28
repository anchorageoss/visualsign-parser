#!/usr/bin/env bash
# Post-deploy smoke test for the live VisualSign parser, dev/staging or prod.
#
# Runs a known Solana V0 transaction (referencing address lookup tables) through
# the deployed parser via `turnkey-client verify` and asserts BOTH:
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
# When the container image is unavailable (e.g. not pullable without registry
# auth, or a pinned tag that does not exist), point
# --turnkey-client-path at a local fallback: an executable client binary, or a
# turnkey-client source dir that is built (`make build`) and run from
# bin/visualsign-turnkeyclient.
#
# Usage: smoke.sh [--target dev|prod] [--turnkey-client-path <binary-or-source-dir>]
#                 [--turnkey-client-version <tag>] [--quiet]
#
# Flags:
#   --target dev|prod           which deployment to smoke (default dev, which
#                               also covers staging: they share one Turnkey app
#                               and org). Picks the endpoint path AND the default
#                               org + key: dev routes to /visualsign-dev via
#                               `--dev-path`, prod to the canonical /visualsign.
#                               Pointing --target prod at the dev org (or vice
#                               versa) smokes an endpoint that doesn't serve that
#                               org, so change org/key and target together.
#   --turnkey-client-path P     local client used only when the container image
#                               is unavailable (or VSP_SMOKE_TURNKEY_CLIENT_PATH)
#   --turnkey-client-version T  container image tag to pull (default: latest);
#                               pin an approved version in CI (or via
#                               VSP_SMOKE_CLIENT_VERSION)
#   --quiet, -q                 suppress the client's output on success (failures
#                               stay verbose); default shows it
#
# Env (the target's org id is REQUIRED, the rest are optional):
#   VSP_SMOKE_ORG_DEV             org id for --target dev (dev + staging share it)
#   VSP_SMOKE_ORG_PROD            org id for --target prod
#   VSP_SMOKE_ORG                 org id for whichever target is selected; wins
#                                 over the two above (this is what CI passes)
#   VSP_SMOKE_HOST                 API host    (default https://api.turnkey.com)
#   VSP_SMOKE_TARGET              dev|prod (same as --target)
#   VSP_SMOKE_KEY                 key name under ~/.config/turnkey/keys/<key>.{public,private}
#                                 (default: dev for --target dev, default for prod)
#
# The org ids are deliberately not defaulted in this script: the repo is public.
# Keep them in CI secrets and in a private local env file, not here.
#   VSP_SMOKE_CLIENT_VERSION      container image tag (same as --turnkey-client-version)
#   TURNKEY_CLIENT                how to invoke the client (overrides all resolution)
#   VSP_SMOKE_TURNKEY_CLIENT_PATH local fallback path (same as --turnkey-client-path)
#
# `~/.config/turnkey/keys/<name>.{public,private}` is a distinct local credential
# store from the one `tvc login` populates (`~/.config/turnkey/orgs/<org>/api_key.json`)
# — the two are not interchangeable, and `tvc login` alone will not satisfy
# VSP_SMOKE_KEY. If your org's credential only exists in the `orgs/` form, convert
# it with `scripts/import-turnkey-api-key.sh --org <org-name>` (matches an
# `[orgs.<org-name>]` entry in ~/.config/turnkey/tvc.config.toml), then point
# VSP_SMOKE_KEY at the resulting key name.
#
# Requires `jq` on PATH (parses the client's JSON response for the assertions below).
#
# Exit: 0 = rendered + verified (pass) OR endpoint unreachable (skip; not ours);
#       1 = endpoint up but parser failed to render / verify / assertions failed;
#       2 = smoke could not run the client (e.g. missing/unpullable image or
#           binary) — a harness failure, never treated as a pass.
set -euo pipefail

HOST="${VSP_SMOKE_HOST:-https://api.turnkey.com}"
# Org/key defaults depend on --target, so they are resolved after arg parsing;
# an explicit env value still wins over the target's default.
ORG="${VSP_SMOKE_ORG:-}"
KEY="${VSP_SMOKE_KEY:-}"
TARGET="${VSP_SMOKE_TARGET:-dev}"
CLIENT_PATH="${VSP_SMOKE_TURNKEY_CLIENT_PATH:-}"
CLIENT_VERSION="${VSP_SMOKE_CLIENT_VERSION:-latest}"
QUIET=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --target)
      [ "$#" -ge 2 ] || { echo "--target requires a value" >&2; exit 2; }
      TARGET="$2"; shift 2 ;;
    --target=*) TARGET="${1#*=}"; shift ;;
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
      echo "usage: smoke.sh [--target dev|prod] [--turnkey-client-path <binary-or-source-dir>] [--turnkey-client-version <tag>] [--quiet]" >&2
      exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

# Resolve the target's endpoint path + default org/key. dev/staging share one
# Turnkey app and org, so `dev` covers both. An unknown target is a usage error,
# never a silent fall back to dev: that would smoke the wrong deployment and
# report it as a pass.
#
# Org ids are NOT baked in. This repo is public and the ids identify Anchorage's
# Turnkey orgs, so they come from the environment (CI secrets, or a private local
# env file); the deploy runbook lists which id goes with which target.
case "$TARGET" in
  dev)
    ORG="${ORG:-${VSP_SMOKE_ORG_DEV:-}}"
    KEY="${KEY:-dev}"
    PATH_ARGS=(--dev-path) ;;
  prod)
    ORG="${ORG:-${VSP_SMOKE_ORG_PROD:-}}"
    KEY="${KEY:-default}"
    PATH_ARGS=() ;;
  *) echo "--target must be dev or prod (got: $TARGET)" >&2; exit 2 ;;
esac
if [ -z "$ORG" ]; then
  # Exit 2, not 1: nothing was smoked, so this is a harness/config failure and
  # must not be reportable as either a pass or a parser regression.
  case "$TARGET" in
    dev) var=VSP_SMOKE_ORG_DEV ;;
    *) var=VSP_SMOKE_ORG_PROD ;;
  esac
  echo "ERROR: no organization id for --target $TARGET; set $var (or VSP_SMOKE_ORG). See the parser_app deploy runbook for the id." >&2
  exit 2
fi
IMAGE="ghcr.io/anchorageoss/visualsign-turnkeyclient:${CLIENT_VERSION}"
# The client resolves its key pair from $HOME/.config/turnkey/keys, so the mount
# target has to match the container's HOME (not /root, which it never looks in).
# Two further wrinkles, both of which this invocation handles:
#   - run as the invoking uid, so a key file with sane 0600 permissions is
#     readable. The image's own user is `nonroot`, which cannot read a key owned
#     by you unless you widen it to 0644; a prod credential should not need that.
#   - mount under a neutral HOME rather than the image's /home/nonroot, which is
#     not traversable by any other uid, so --user alone would still hit EACCES.
CONTAINER_CLIENT="docker run --rm --user $(id -u):$(id -g) -e HOME=/tkhome -v $HOME/.config/turnkey/keys:/tkhome/.config/turnkey/keys:ro $IMAGE"

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

echo "smoking target=$TARGET org=$ORG key=$KEY" >&2
set +e
OUT="$($CLIENT verify "${PATH_ARGS[@]}" --host "$HOST" --organization-id "$ORG" \
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
  # A genuine transport/network error where we never even connected is a
  # pre-existing outage, not the deploy's fault -> skip. Deliberately excludes
  # bare timeout/EOF/context-deadline below: those equally describe a
  # connection that WAS established to a hung or crashed enclave, which is
  # exactly the failure this smoke check exists to catch, so they must not be
  # classified as a benign outage.
  if grep -qiE \
    'connection refused|connection reset|no such host|dial tcp|tls handshake|network is unreachable|server misbehaving|temporary failure in name resolution' \
    "$ERRFILE"; then
    echo "SKIP: dev endpoint unreachable / outage — not a regression" >&2
    exit 0
  fi
  # A timeout, dropped connection (EOF), or context-deadline reaching an
  # endpoint we could otherwise connect to is ambiguous with a hung or
  # crashed enclave right after set-live -> treat as a possible regression,
  # not a skip, so a broken deploy can't hide behind an outage classification.
  if grep -qiE 'timeout|\bEOF\b|context deadline' "$ERRFILE"; then
    echo "FAIL: endpoint reachable but request timed out / connection dropped after connecting (possible deploy regression, not an outage)" >&2
    exit 1
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
echo "PASS: turnkey-client verify succeeded on target=$TARGET (org=$ORG); V0+ALT rendered ($chars chars, no \"Cannot render V0\"); attestation + signature cryptographically verified (executed in AWS Nitro enclave); moduleId=$module"
