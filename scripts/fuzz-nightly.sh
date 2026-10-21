#!/usr/bin/env bash
# Maintenance for the nightly toolchain pinned for cargo-fuzz.
#   check-needed  fail if stable can now build the fuzz targets (the nightly pin may be unnecessary)
#   bump          move the pin to the newest nightly that builds and briefly fuzzes every target
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
FUZZ_DIR=src/chain_parsers/visualsign-solana/fuzz
PIN_FILE=$FUZZ_DIR/rust-toolchain.toml
current() { sed -n 's/^channel = "\(nightly-[0-9-]*\)"/\1/p' "$PIN_FILE"; }

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

# Build and briefly fuzz every target on one nightly; nonzero if any step fails.
validate() {
    local nightly=$1 targets t
    (
        cd "$FUZZ_DIR" || exit 1
        cargo "+$nightly" fuzz build || exit 1
        targets=$(cargo "+$nightly" fuzz list) || exit 1
        for t in $targets; do
            cargo "+$nightly" fuzz run "$t" -- -max_total_time=20 || exit 1
        done
    )
}

bump() {
    local cur candidate back tried=0
    cur=$(current)
    # Newest first; the pin moves only to a nightly that passed validate, so a rerun
    # never finds an unvalidated pin already in place.
    for back in 0 1 2 3 4 5 6; do
        candidate="nightly-$(date -u -d "$back days ago" +%F)"
        [[ $candidate > $cur ]] || break
        rustup toolchain install "$candidate" --profile minimal >/dev/null 2>&1 || continue
        tried=$((tried + 1))
        if validate "$candidate"; then
            sed -i "s/$cur/$candidate/" "$PIN_FILE"
            echo "bumped $cur -> $candidate"
            return 0
        fi
        echo "warning: $candidate did not build or fuzz cleanly; trying an older nightly" >&2
    done
    if ((tried > 0)); then
        echo "error: no nightly newer than $cur built and fuzzed cleanly" >&2
        return 1
    fi
    echo "already on $cur"
}

case "${1:-}" in
    check-needed) check_needed ;;
    bump) bump ;;
    *) echo "usage: $0 check-needed|bump" >&2; exit 2 ;;
esac
