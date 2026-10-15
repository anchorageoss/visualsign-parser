#!/usr/bin/env bash
# Maintenance for the nightly toolchain pinned for cargo-fuzz.
#   check-needed  fail if stable can now build the fuzz targets (the nightly pin may be unnecessary)
#   bump          move the pin to the newest nightly that builds and briefly fuzzes every target
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
FUZZ_DIR=src/chain_parsers/visualsign-solana/fuzz
PIN_FILES=(.github/workflows/fuzz-solana.yml "$FUZZ_DIR/rust-toolchain.toml")
current() { sed -n 's/^channel = "\(nightly-[0-9-]*\)"/\1/p' "$FUZZ_DIR/rust-toolchain.toml"; }

check_needed() {
    local out
    if out=$(cd "$FUZZ_DIR" && cargo +stable fuzz build 2>&1); then
        echo "error: stable builds the fuzz targets; drop the nightly pin" >&2
    elif grep -q "only accepted on the nightly compiler" <<<"$out"; then
        echo "ok: nightly is still required (cargo-fuzz passes -Zsanitizer=address, which stable rejects)"
        return 0
    else
        echo "error: stable failed for a reason other than nightly-only flags:" >&2
        tail -n 20 <<<"$out" >&2
    fi
    return 1
}

bump() {
    local cur new="" candidate back
    cur=$(current)
    for back in 0 1 2 3 4 5 6; do
        candidate="nightly-$(date -u -d "$back days ago" +%F)"
        if rustup toolchain install "$candidate" --profile minimal >/dev/null 2>&1; then new=$candidate; break; fi
    done
    [[ -n $new ]] || { echo "error: no nightly installable from the last week" >&2; return 1; }
    [[ $new > $cur ]] || { echo "already on $cur"; return 0; }
    sed -i "s/$cur/$new/" "${PIN_FILES[@]}"
    (cd "$FUZZ_DIR" && cargo "+$new" fuzz build \
        && for t in $(cargo "+$new" fuzz list); do cargo "+$new" fuzz run "$t" -- -max_total_time=20; done)
    echo "bumped $cur -> $new"
}

case "${1:-}" in
    check-needed) check_needed ;;
    bump) bump ;;
    *) echo "usage: $0 check-needed|bump" >&2; exit 2 ;;
esac
