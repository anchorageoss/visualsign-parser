#!/usr/bin/env bash
# Materialize a `tvc login`-provisioned org credential
# (~/.config/turnkey/orgs/<org>/api_key.json, format: {public_key, private_key,
# curve}) into the local API-key-store format that scripts/smoke.sh (via
# turnkey-client) and the `turnkey`/tkcli reference the docs at
# https://github.com/tkhq/go-sdk pkg/store/local expects:
#   ~/.config/turnkey/keys/<name>.public   — raw hex public key
#   ~/.config/turnkey/keys/<name>.private  — "<hex private key>:<curve>"
#   ~/.config/turnkey/keys/<name>.meta     — {"name","organizations","public_key","scheme"}
#
# `tvc login` and `smoke.sh` read two different, non-interchangeable local
# credential stores; this bridges the gap so a workstation that's only ever
# run `tvc login` can populate a key smoke.sh can authenticate with, without
# hand-deriving the tkcli file format.
#
# Usage: import-turnkey-api-key.sh --org <org-name> [--key-name <name>]
#
# Flags:
#   --org <name>       org directory name under ~/.config/turnkey/orgs/
#                       (matches an [orgs.<name>] entry in tvc.config.toml)
#   --key-name <name>  name for the resulting keys/<name>.* files
#                       (default: same as --org; this is the value to pass as
#                       VSP_SMOKE_KEY / smoke.sh's --key-name equivalent)
set -euo pipefail

ORG=""
KEY_NAME=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --org)
      [ "$#" -ge 2 ] || { echo "--org requires a value" >&2; exit 2; }
      ORG="$2"; shift 2 ;;
    --org=*) ORG="${1#*=}"; shift ;;
    --key-name)
      [ "$#" -ge 2 ] || { echo "--key-name requires a value" >&2; exit 2; }
      KEY_NAME="$2"; shift 2 ;;
    --key-name=*) KEY_NAME="${1#*=}"; shift ;;
    -h | --help)
      echo "usage: import-turnkey-api-key.sh --org <org-name> [--key-name <name>]" >&2
      exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

[ -n "$ORG" ] || { echo "ERROR: --org is required" >&2; exit 2; }
[ -n "$KEY_NAME" ] || KEY_NAME="$ORG"

# Both values become path components below; reject anything that could escape
# the intended orgs/keys directories (e.g. --key-name ../../etc/foo).
case "$ORG" in
  */* | .*) echo "ERROR: --org must not contain '/' or start with '.': $ORG" >&2; exit 2 ;;
esac
case "$KEY_NAME" in
  */* | .*) echo "ERROR: --key-name must not contain '/' or start with '.': $KEY_NAME" >&2; exit 2 ;;
esac

command -v jq >/dev/null 2>&1 || { echo "ERROR: jq is required" >&2; exit 2; }

SRC="$HOME/.config/turnkey/orgs/$ORG/api_key.json"
[ -f "$SRC" ] || {
  echo "ERROR: no such org credential: $SRC" >&2
  echo "       (run \`tvc login\` first, or check ~/.config/turnkey/tvc.config.toml for the org name)" >&2
  exit 2
}

KEYS_DIR="$HOME/.config/turnkey/keys"
mkdir -p "$KEYS_DIR"
PUB_FILE="$KEYS_DIR/$KEY_NAME.public"
PRIV_FILE="$KEYS_DIR/$KEY_NAME.private"
META_FILE="$KEYS_DIR/$KEY_NAME.meta"

for f in "$PUB_FILE" "$PRIV_FILE" "$META_FILE"; do
  [ ! -e "$f" ] || { echo "ERROR: refusing to overwrite existing file: $f" >&2; exit 2; }
done

PUBLIC_KEY="$(jq -e -r '.public_key' "$SRC")" || { echo "ERROR: $SRC missing/null public_key" >&2; exit 2; }
PRIVATE_KEY="$(jq -e -r '.private_key' "$SRC")" || { echo "ERROR: $SRC missing/null private_key" >&2; exit 2; }
CURVE="$(jq -e -r '.curve' "$SRC")" || { echo "ERROR: $SRC missing/null curve" >&2; exit 2; }

case "$CURVE" in
  p256) SCHEME="SIGNATURE_SCHEME_TK_API_P256" ;;
  secp256k1) SCHEME="SIGNATURE_SCHEME_TK_API_SECP256K1" ;;
  ed25519) SCHEME="SIGNATURE_SCHEME_TK_API_ED25519" ;;
  *) echo "ERROR: unsupported curve in $SRC: $CURVE" >&2; exit 2 ;;
esac

# Look up the org id from tvc.config.toml so the meta file's `organizations`
# field matches what tkcli itself would write; fall back to the org name if
# the config or entry isn't present (metadata only, not read by smoke.sh).
CONFIG="$HOME/.config/turnkey/tvc.config.toml"
ORG_ID="$ORG"
if [ -f "$CONFIG" ]; then
  found="$(awk -v org="[orgs.$ORG]" '
    $0 == org { in_section=1; next }
    /^\[/ { in_section=0 }
    in_section && /^id[[:space:]]*=/ {
      sub(/^id[[:space:]]*=[[:space:]]*"/, "");
      sub(/"[[:space:]]*$/, "");
      print;
      exit
    }
  ' "$CONFIG")"
  [ -z "$found" ] || ORG_ID="$found"
fi

umask 077
printf '%s' "$PUBLIC_KEY" > "$PUB_FILE"
chmod 0644 "$PUB_FILE"
printf '%s:%s' "$PRIVATE_KEY" "$CURVE" > "$PRIV_FILE"
chmod 0600 "$PRIV_FILE"
jq -n --arg name "$KEY_NAME" --arg org_id "$ORG_ID" --arg pub "$PUBLIC_KEY" --arg scheme "$SCHEME" \
  '{name: $name, organizations: [$org_id], public_key: $pub, scheme: $scheme}' > "$META_FILE"
chmod 0600 "$META_FILE"

echo "wrote $PUB_FILE"
echo "wrote $PRIV_FILE"
echo "wrote $META_FILE"
echo "run smoke.sh with: VSP_SMOKE_KEY=$KEY_NAME ./scripts/smoke.sh ..."
