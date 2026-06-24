#!/usr/bin/env bash
# Post-deploy smoke test for the live dev VisualSign parser.
#
# Parses a known Solana V0 transaction that references address lookup tables
# through the deployed parser (the /visualsign-dev endpoint) and asserts it
# RENDERS — a regression guard for the "Cannot render V0 ... refusing to display
# a partial transaction" failure.
#
# Drives the published turnkey-client CONTAINER (no Go toolchain needed); its
# stdout is the response JSON, so assertions are pure `jq`.
#
# Env (all optional; defaults target the dev endpoint/app):
#   VSP_SMOKE_HOST   API host                  (default https://api.turnkey.com)
#   VSP_SMOKE_ORG    organization id
#   VSP_SMOKE_APP    expected enclaveApp (the app wired to /visualsign-dev)
#   VSP_SMOKE_KEY    key name under ~/.config/turnkey/keys/<key>.{public,private}
#   TURNKEY_CLIENT   how to invoke the client (default: the GHCR container)
#
# Exit: 0 = rendered (pass) OR endpoint unreachable (skip; not our regression);
#       1 = endpoint up but parser failed to render / assertions failed.
set -euo pipefail

HOST="${VSP_SMOKE_HOST:-https://api.turnkey.com}"
ORG="${VSP_SMOKE_ORG:-d7f51c3d-fb9d-47c1-9b2e-a02b1cd5ff14}"
APP="${VSP_SMOKE_APP:-e349edd8-2a25-4083-922d-592ebded9acf}"
KEY="${VSP_SMOKE_KEY:-dev}"
CLIENT="${TURNKEY_CLIENT:-docker run --rm -v $HOME/.config/turnkey/keys:/root/.config/turnkey/keys:ro ghcr.io/anchorageoss/visualsign-turnkeyclient:latest}"

DIR="$(cd "$(dirname "$0")/.." && pwd)"
PAYLOAD="$(tr -d '[:space:]' < "$DIR/testdata/solana_v0_alt.b64")"
ERRFILE="$(mktemp)"
trap 'rm -f "$ERRFILE"' EXIT

set +e
OUT="$($CLIENT parse --dev-path --host "$HOST" --organization-id "$ORG" \
  --key-name "$KEY" --unsigned-payload "$PAYLOAD" 2>"$ERRFILE")"
RC=$?
set -e

if [ "$RC" -ne 0 ]; then
  # Abort guard: only a parser-level rejection (endpoint reachable, non-OK
  # status) is a regression. A transport/connection error is a pre-existing
  # outage and must not be blamed on the deploy.
  if grep -q "non-OK status" "$ERRFILE"; then
    echo "FAIL: deployed parser rejected a tx it should render (regression):" >&2
    cat "$ERRFILE" >&2
    exit 1
  fi
  echo "SKIP: dev endpoint unreachable / outage — not a regression:" >&2
  cat "$ERRFILE" >&2
  exit 0
fi

if ! echo "$OUT" | jq -e --arg app "$APP" '
      (.signablePayload | length > 0)
  and (.signablePayload | contains("Cannot render V0") | not)
  and (.attestations | has("app_attestation") and has("boot_attestation"))
  and (.enclaveApp == $app)
' >/dev/null; then
  echo "FAIL: render assertions failed. Response summary:" >&2
  echo "$OUT" | jq '{signablePayloadLen: (.signablePayload | length), enclaveApp, deploymentLabel, attestations: (.attestations | keys)}' >&2
  exit 1
fi

echo "PASS: V0+ALT rendered ($(echo "$OUT" | jq -r '.signablePayload | length') chars); enclaveApp=$(echo "$OUT" | jq -r .enclaveApp) deploymentLabel=$(echo "$OUT" | jq -r .deploymentLabel)"
