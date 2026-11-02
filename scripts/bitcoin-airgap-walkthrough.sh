#!/usr/bin/env bash
# Walkthrough: check a PSBT against your own air-gapped watch-only Bitcoin Core
# wallet before signing (VSP-BTC-1 prototype).
#
# Two regtest nodes in temp dirs that are never connected:
#   online - holds alice's wallet, mines, builds the PSBT (the coordinator)
#   airgap - networking off, no blocks, a watch-only wallet holding only
#            alice's PUBLIC descriptors. It is the only node parser_cli reads.
#
# Usage (from the repo root):
#   (cd src && cargo build -p parser_cli --no-default-features --features bitcoin)
#   BITCOIND=/path/to/bitcoind BITCOIN_CLI=/path/to/bitcoin-cli \
#   PARSER_CLI=$CARGO_TARGET_DIR/debug/parser_cli scripts/bitcoin-airgap-walkthrough.sh
set -euo pipefail

BITCOIND=${BITCOIND:-bitcoind}
BITCOIN_CLI=${BITCOIN_CLI:-bitcoin-cli}
PARSER_CLI=${PARSER_CLI:-parser_cli}
ONLINE_PORT=${ONLINE_PORT:-18543}
AIRGAP_PORT=${AIRGAP_PORT:-18643}
for v in BITCOIND BITCOIN_CLI PARSER_CLI; do [[ ${!v} == */* ]] && printf -v "$v" %s "$(realpath "${!v}")"; done
for tool in jq xxd base64; do command -v "$tool" >/dev/null || { echo "needs $tool" >&2; exit 1; }; done

WORK=$(mktemp -d)
ONLINE=$WORK/online
AIRGAP=$WORK/airgap
mkdir -p "$ONLINE" "$AIRGAP"
cd "$WORK"

on() { "$BITCOIN_CLI" -regtest -datadir="$ONLINE" -rpcport="$ONLINE_PORT" "$@"; }
ag() { "$BITCOIN_CLI" -regtest -datadir="$AIRGAP" -rpcport="$AIRGAP_PORT" "$@"; }
cleanup() {
  on stop >/dev/null 2>&1 || true
  ag stop >/dev/null 2>&1 || true
  while on getblockchaininfo >/dev/null 2>&1 || ag getblockchaininfo >/dev/null 2>&1; do sleep 0.2; done
  cd / && rm -rf "$WORK"
}
trap cleanup EXIT


# 1. Start both nodes. The air-gapped one never listens and has networking off.
"$BITCOIND" -regtest -datadir="$ONLINE" -rpcport="$ONLINE_PORT" -port=$((ONLINE_PORT + 1)) \
  -listen=0 -fallbackfee=0.0002 -daemonwait >/dev/null
"$BITCOIND" -regtest -datadir="$AIRGAP" -rpcport="$AIRGAP_PORT" -port=$((AIRGAP_PORT + 1)) \
  -listen=0 -networkactive=0 -daemonwait >/dev/null

# 2. Online: alice mines 101 blocks and builds a PSBT paying bob 1 BTC, with
#    change at output 1.
on createwallet alice >/dev/null
on createwallet bob >/dev/null
MINING=$(on -rpcwallet=alice getnewaddress "" bech32)
on generatetoaddress 101 "$MINING" >/dev/null
PAYEE=$(on -rpcwallet=bob getnewaddress "" bech32)
PSBT=$(on -rpcwallet=alice -named walletcreatefundedpsbt inputs='[]' \
  outputs="[{\"$PAYEE\":1.0}]" options='{"changePosition":1}' | jq -r .psbt)

# 3. Air-gapped: a blank watch-only wallet with alice's public descriptors
#    (`listdescriptors false`), and two labels of its own.
ag -named createwallet wallet_name=airgap disable_private_keys=true blank=true >/dev/null
DESCRIPTORS=$(on -rpcwallet=alice listdescriptors false | jq -c '[.descriptors[]
  | {desc, timestamp: "now", active, internal, range, next_index: (.next_index // .next // 0)}
  | with_entries(select(.value != null))]')
ag -rpcwallet=airgap importdescriptors "$DESCRIPTORS" | jq -e 'all(.success)' >/dev/null
ag -rpcwallet=airgap setlabel "$MINING" "Mining rewards"
ag -rpcwallet=airgap setlabel "$PAYEE" "Bob"
echo "airgap node: $(ag getblockchaininfo | jq .blocks) blocks, $(ag getconnectioncount) peers"

# The PSBT crosses the air gap as a file (USB stick, QR codes, ...).
echo "$PSBT" >honest.psbt
CHECK=("$PARSER_CLI" bitcoin wallet-check --network regtest
  --rpc-url "http://127.0.0.1:$AIRGAP_PORT" --rpc-cookie "$AIRGAP/regtest/.cookie"
  --wallet airgap --account-name Savings)

echo; echo "== honest PSBT"
"${CHECK[@]}" --psbt @honest.psbt

# 4. Fake change: keep output 1's bip32_derivation (alice's change path) but
#    pay bob's key at the same path. Both are 22-byte P2WPKH scripts, so a byte
#    swap in the serialized PSBT is enough.
CHANGE_PATH=$(on decodepsbt "$PSBT" | jq -r '.outputs[1].bip32_derivs[0].path')
INDEX=${CHANGE_PATH##*/}
BOB_CHANGE_DESC=$(on -rpcwallet=bob listdescriptors false |
  jq -r '.descriptors[] | select(.internal and (.desc | startswith("wpkh("))) | .desc')
BOB_CHANGE=$(on deriveaddresses "$BOB_CHANGE_DESC" "[$INDEX,$INDEX]" | jq -r '.[0]')
OLD=$(on decodepsbt "$PSBT" | jq -r '.tx.vout[1].scriptPubKey.hex')
NEW=$(on validateaddress "$BOB_CHANGE" | jq -r .scriptPubKey)
PSBT_HEX=$(echo "$PSBT" | base64 -d | xxd -p | tr -d '\n')
[ "$(grep -o "$OLD" <<<"$PSBT_HEX" | wc -l)" = 1 ] || { echo "change script not unique" >&2; exit 1; }
sed "s/$OLD/$NEW/" <<<"$PSBT_HEX" | xxd -r -p | base64 -w0 >fake.psbt

echo; echo "== fake-change PSBT (output 1 pays bob, still claims $CHANGE_PATH)"
"${CHECK[@]}" --psbt @fake.psbt

# 5. --compare: what a backend sent vs what your node says. The honest backend
#    sends the node's own metadata. A compromised one adds bob's xpub as a
#    second "Savings" account under alice's fingerprint and origin, which makes
#    the fake change verify against the request metadata alone.
"${CHECK[@]}" --psbt @honest.psbt --output json | jq .metadata >backend-honest.json
BOB_XPUB=$(sed -E 's/.*\]([^/]+)\/.*/\1/' <<<"$BOB_CHANGE_DESC")
jq --arg xpub "$BOB_XPUB" '.accounts += [.accounts[] | select(.script_type == "p2wpkh") | .xpub = $xpub]' \
  backend-honest.json >backend-evil.json

echo; echo "== --compare, honest backend, honest PSBT"
"${CHECK[@]}" --psbt @honest.psbt --compare backend-honest.json | sed -n '/^Compare:/,$p'
echo; echo "== --compare, compromised backend, fake-change PSBT"
"${CHECK[@]}" --psbt @fake.psbt --compare backend-evil.json | sed -n '/^Compare:/,$p'
