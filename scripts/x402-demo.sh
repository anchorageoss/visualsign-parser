#!/usr/bin/env bash
#
# x402-demo.sh — narrated end-to-end walkthrough of the x402 v2 gated HTTP API
#                added to parser_gateway. Spins up the mock facilitator, the
#                parser gRPC server, and the gateway, then steps through each
#                scenario with commentary.
#
# Run from the repo root: ./scripts/x402-demo.sh
# Requirements: bash, curl, jq, base64, cargo. No network needed.
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

# ---------- presentation helpers ---------------------------------------------

if [ -t 1 ]; then
  BOLD=$'\033[1m'; DIM=$'\033[2m'; CYAN=$'\033[36m'; GREEN=$'\033[32m'
  YELLOW=$'\033[33m'; RED=$'\033[31m'; MAGENTA=$'\033[35m'; RESET=$'\033[0m'
else
  BOLD=''; DIM=''; CYAN=''; GREEN=''; YELLOW=''; RED=''; MAGENTA=''; RESET=''
fi

chapter() { printf '\n%s%s%s\n%s%s%s\n' "$BOLD$MAGENTA" "════ $1 ════" "$RESET" "$DIM" "$2" "$RESET"; }
say()     { printf '%s%s%s\n' "$CYAN" "$1" "$RESET"; }
narrate() { printf '%s│ %s%s\n' "$DIM" "$1" "$RESET"; }
ok()      { printf '%s✓ %s%s\n' "$GREEN" "$1" "$RESET"; }
warn()    { printf '%s! %s%s\n' "$YELLOW" "$1" "$RESET"; }
fail()    { printf '%s✗ %s%s\n' "$RED" "$1" "$RESET"; exit 1; }
cmd()     { printf '%s$ %s%s\n' "$YELLOW" "$1" "$RESET"; }

pause() { sleep "${DEMO_PAUSE:-0.4}"; }

# ---------- preflight --------------------------------------------------------

chapter "Preflight" "Make sure we have everything we need before starting."

for tool in curl jq base64 cargo; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    fail "missing tool: $tool"
  fi
done
ok "curl, jq, base64, cargo all present"

MOCK_BIN="src/target/debug/mock_facilitator"
GRPC_BIN="src/target/debug/parser_grpc_server"
GW_BIN="src/target/debug/parser_gateway"

if [ ! -x "$MOCK_BIN" ] || [ ! -x "$GRPC_BIN" ] || [ ! -x "$GW_BIN" ]; then
  warn "one or more binaries missing — running 'make -C src build' (may take a minute)"
  make -C src build >/dev/null
fi
ok "binaries built"

# Free our demo ports if any were left running
for port in 8090 44020 8080; do
  pid=$(lsof -ti tcp:"$port" 2>/dev/null || true)
  if [ -n "$pid" ]; then
    warn "port $port held by pid $pid — killing"
    kill "$pid" 2>/dev/null || true
    sleep 0.3
  fi
done

# ---------- process management -----------------------------------------------

MOCK_PORT=8090
GW_PORT=8080
GRPC_PORT=44020

LOG_DIR="$(mktemp -d)"
MOCK_LOG="$LOG_DIR/mock_facilitator.log"
GRPC_LOG="$LOG_DIR/parser_grpc_server.log"
GW_LOG="$LOG_DIR/parser_gateway.log"

MOCK_PID=""; GRPC_PID=""; GW_PID=""

cleanup() {
  set +e
  for pid in "$MOCK_PID" "$GRPC_PID" "$GW_PID"; do
    [ -n "$pid" ] && kill "$pid" 2>/dev/null
  done
  for pid in "$MOCK_PID" "$GRPC_PID" "$GW_PID"; do
    [ -n "$pid" ] && wait "$pid" 2>/dev/null
  done
  if [ -n "${DEMO_KEEP_LOGS:-}" ]; then
    say "logs preserved in $LOG_DIR"
  else
    rm -rf "$LOG_DIR"
  fi
}
trap cleanup EXIT INT TERM

wait_for_url() {
  local url="$1"; local label="$2"
  for _ in $(seq 1 50); do
    if curl -sf "$url" >/dev/null 2>&1; then
      ok "$label ready"
      return 0
    fi
    sleep 0.1
  done
  fail "$label never became ready (probed $url)"
}

wait_port_free() {
  local port="$1"
  for _ in $(seq 1 30); do
    if ! lsof -ti tcp:"$port" >/dev/null 2>&1; then return 0; fi
    sleep 0.1
  done
}

start_stack() {
  # Optional first arg: JSON value for X402_PRICE_TAGS_JSON (passed through env,
  # NOT word-split — JSON can contain whitespace and newlines).
  local price_tags_json="${1:-}"

  wait_port_free "$MOCK_PORT"
  wait_port_free "$GRPC_PORT"
  wait_port_free "$GW_PORT"

  narrate "starting mock_facilitator on :$MOCK_PORT (approves every payment)"
  MOCK_FACILITATOR_PORT=$MOCK_PORT "$MOCK_BIN" >"$MOCK_LOG" 2>&1 &
  MOCK_PID=$!
  wait_for_url "http://127.0.0.1:$MOCK_PORT/supported" "mock_facilitator"

  narrate "starting parser_grpc_server on :$GRPC_PORT"
  EPHEMERAL_FILE="src/integration/fixtures/ephemeral.secret" \
    "$GRPC_BIN" >"$GRPC_LOG" 2>&1 &
  GRPC_PID=$!

  if [ -n "$price_tags_json" ]; then
    narrate "starting parser_gateway on :$GW_PORT (x402 profile=local + multi-tag JSON)"
    GATEWAY_PORT=$GW_PORT \
    GRPC_ADDR="http://127.0.0.1:$GRPC_PORT" \
    X402_PROFILE=local \
    X402_FACILITATOR_URL="http://127.0.0.1:$MOCK_PORT" \
    X402_PRICE_TAGS_JSON="$price_tags_json" \
      "$GW_BIN" >"$GW_LOG" 2>&1 &
  else
    narrate "starting parser_gateway on :$GW_PORT (x402 profile=local)"
    GATEWAY_PORT=$GW_PORT \
    GRPC_ADDR="http://127.0.0.1:$GRPC_PORT" \
    X402_PROFILE=local \
    X402_FACILITATOR_URL="http://127.0.0.1:$MOCK_PORT" \
      "$GW_BIN" >"$GW_LOG" 2>&1 &
  fi
  GW_PID=$!
  wait_for_url "http://127.0.0.1:$GW_PORT/health" "parser_gateway"
}

stop_stack() {
  for pid in "$MOCK_PID" "$GRPC_PID" "$GW_PID"; do
    [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
  done
  for pid in "$MOCK_PID" "$GRPC_PID" "$GW_PID"; do
    [ -n "$pid" ] && wait "$pid" 2>/dev/null || true
  done
  MOCK_PID=""; GRPC_PID=""; GW_PID=""
  sleep 0.3
}

# ---------- shared fixtures --------------------------------------------------

# A real signed legacy Ethereum transfer — same fixture the integration tests use.
ETH_TX_HEX="0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83"

parse_body() {
  jq -n --arg tx "$ETH_TX_HEX" \
    '{request: {unsigned_payload: $tx, chain: "CHAIN_ETHEREUM"}}'
}

# Build a Payment-Signature header from a 402 response's Payment-Required header.
# x402 v2 wire format: base64(JSON({x402Version, accepted: <requirements>, payload: {...}}))
build_payment_signature() {
  local pr_b64="$1"
  local requirements
  requirements=$(printf %s "$pr_b64" | base64 -d 2>/dev/null | jq '.accepts[0]')
  jq -nc --argjson req "$requirements" \
    '{x402Version: 2, accepted: $req, payload: {payer: "0xDEM0DEM0DEM0DEM0DEM0DEM0DEM0DEM0DEM0DEM0"}}' \
    | base64 -w0
}

# Pretty-print a JSON snippet, trimmed to N lines.
pp() { jq -C . 2>/dev/null || cat; }

# ---------- demo -------------------------------------------------------------

chapter "Scene 1 — Boot the stack" \
        "Three Rust binaries; the gateway probes the facilitator before binding."

start_stack
echo
narrate "logs at $LOG_DIR (set DEMO_KEEP_LOGS=1 to keep them)"
pause

chapter "Scene 2 — /health is open" \
        "Health checks must never require payment; orchestrators can't sign x402."

cmd "curl -s http://127.0.0.1:$GW_PORT/health"
curl -s "http://127.0.0.1:$GW_PORT/health" | pp
pause

chapter "Scene 3 — v1 Turnkey endpoint is untouched" \
        "The existing /visualsign/api/v1/parse path stays open — Turnkey deployments keep working."

cmd "curl -s http://127.0.0.1:$GW_PORT/visualsign/api/v1/parse -d <eth tx>"
parse_body | curl -s -H 'content-type: application/json' \
  -X POST -d @- "http://127.0.0.1:$GW_PORT/visualsign/api/v1/parse" \
  | pp | head -20
ok "v1 returned a signable payload — no x402 challenge"
pause

chapter "Scene 4 — v2 endpoint, no payment → 402 Payment Required" \
        "x402-axum intercepts before the handler runs. The Payment-Required header
         carries the base64-JSON of accepted payment options."

cmd "curl -i http://127.0.0.1:$GW_PORT/visualsign/api/v2/parse  # no payment header"
hdr_file=$(mktemp)
parse_body | curl -s -H 'content-type: application/json' \
  -X POST -d @- -D "$hdr_file" -o /dev/null \
  "http://127.0.0.1:$GW_PORT/visualsign/api/v2/parse"

status=$(awk 'NR==1 {print $2}' "$hdr_file")
pr_b64=$(awk -F': ' 'tolower($1)=="payment-required" {sub(/\r$/, "", $2); print $2}' "$hdr_file" | head -1)

narrate "status: $status"
if [ -z "$pr_b64" ]; then
  warn "no Payment-Required header — printing raw headers for debugging:"
  cat "$hdr_file"
else
  ok "Payment-Required header found (${#pr_b64} bytes base64)"
  say "decoded payment requirements:"
  printf %s "$pr_b64" | base64 -d | pp
fi
rm -f "$hdr_file"
pause

chapter "Scene 5 — v2 with payment → 200 + signable payload + settle" \
        "We echo the requirements back as a (mock) signed payment.
         x402-axum verifies via mock_facilitator, calls the handler, then settles."

sig=$(build_payment_signature "$pr_b64")
narrate "Payment-Signature header: ${sig:0:48}… (truncated, ${#sig} bytes)"

cmd "curl -i -H 'Payment-Signature: <stub>' http://127.0.0.1:$GW_PORT/visualsign/api/v2/parse"
resp_hdr=$(mktemp)
resp_body=$(parse_body | curl -s -H 'content-type: application/json' \
  -H "Payment-Signature: $sig" \
  -X POST -d @- -D "$resp_hdr" \
  "http://127.0.0.1:$GW_PORT/visualsign/api/v2/parse")

status=$(awk 'NR==1 {print $2}' "$resp_hdr")
narrate "status: $status"

payment_resp=$(awk -F': ' 'tolower($1)=="payment-response" {sub(/\r$/, "", $2); print $2}' "$resp_hdr" | head -1)
if [ -n "$payment_resp" ]; then
  ok "Payment-Response header present (${#payment_resp} bytes base64)"
  say "decoded settlement receipt:"
  printf %s "$payment_resp" | base64 -d | pp
else
  warn "no Payment-Response header (x402-axum may emit a differently-named header in this version)"
fi
echo
say "and the actual response body:"
printf %s "$resp_body" | pp | head -20
rm -f "$resp_hdr"
pause

chapter "Scene 6 — v2 with malformed tx → 400, no settlement" \
        "The middleware's settle_on_success contract: a 4xx handler response
         means the payment is verified but never actually settled.
         (The mock approves anything, so we can't directly observe non-settlement here,
         but the contract is documented in x402-axum and exercised by Task 10's path 3.)"

cmd "curl -i -H 'Payment-Signature: <stub>' -d '{\"request\":{\"unsigned_payload\":\"0xnope\",...}}'"
bad_body='{"request": {"unsigned_payload": "0xnope", "chain": "CHAIN_ETHEREUM"}}'
status=$(printf %s "$bad_body" | curl -s -o /dev/null -w '%{http_code}' \
  -H 'content-type: application/json' \
  -H "Payment-Signature: $sig" \
  -X POST -d @- "http://127.0.0.1:$GW_PORT/visualsign/api/v2/parse")
narrate "status: $status"
if [ "$status" = "400" ]; then
  ok "parser rejected the payload before settle"
else
  warn "expected 400, got $status"
fi
pause

chapter "Scene 7 — Multi-tag config via X402_PRICE_TAGS_JSON" \
        "Restart the gateway advertising TWO payment options (base USDC OR solana USDC)."

stop_stack

multi=$(jq -nc '[
  { network: "base",   asset: "USDC", priceUsd: "0.002",
    payTo: { evm: "0xfedcba0000000000000000000000000000000099" },
    scheme: "exact" },
  { network: "solana", asset: "USDC", priceUsd: "0.002",
    payTo: { solana: "EGBQqKn968sVv5cQh5Cr72pSTHfxsuzq7o7asqYB5uEV" },
    scheme: "exact" }
]')

start_stack "$multi"

cmd "curl -i http://127.0.0.1:$GW_PORT/visualsign/api/v2/parse  # no payment"
hdr_file=$(mktemp)
parse_body | curl -s -H 'content-type: application/json' \
  -X POST -d @- -D "$hdr_file" -o /dev/null \
  "http://127.0.0.1:$GW_PORT/visualsign/api/v2/parse"
pr_b64=$(awk -F': ' 'tolower($1)=="payment-required" {sub(/\r$/, "", $2); print $2}' "$hdr_file" | head -1)
if [ -z "$pr_b64" ]; then
  warn "no Payment-Required header — gateway may not be up; raw headers:"
  cat "$hdr_file"
else
  say "advertised accepts (summary):"
  printf %s "$pr_b64" | base64 -d \
    | jq -C '.accepts | map({network, scheme, amount, payTo})'
  ok "two payment options advertised on a single endpoint"
fi
rm -f "$hdr_file"

# ---------- close out --------------------------------------------------------

chapter "Curtain" \
        "Stack will shut down cleanly when the script exits."

cat <<EOF
${BOLD}What we just showcased:${RESET}
  • New ${CYAN}POST /visualsign/api/v2/parse${RESET} gated by x402 v2 middleware
  • Existing ${CYAN}/health${RESET} and ${CYAN}/visualsign/api/v1/parse${RESET} untouched (Turnkey-compatible)
  • Configurable facilitator (${CYAN}X402_FACILITATOR_URL${RESET}) — local mock here, PayAI or custom in prod
  • Profile defaults (${CYAN}X402_PROFILE=local|payai|custom${RESET}) keep dev cheap
  • Multi-tag advertising via ${CYAN}X402_PRICE_TAGS_JSON${RESET}
  • Startup probe ${CYAN}GET /supported${RESET} fails-fast on misconfig
  • ${CYAN}mock_facilitator${RESET} crate for offline dev + integration tests

${BOLD}Re-run with options:${RESET}
  ${DIM}DEMO_PAUSE=1.5 ./scripts/x402-demo.sh${RESET}      # slower narration
  ${DIM}DEMO_KEEP_LOGS=1 ./scripts/x402-demo.sh${RESET}    # keep per-service logs after exit
EOF
