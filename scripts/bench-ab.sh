#!/usr/bin/env bash
# A/B benchmark two commits on the current machine and write a Markdown report.
#
# Usage: scripts/bench-ab.sh <base-ref> <head-ref> [criterion-filter]
#
# Both sides run back to back on the same machine, so the comparison is fair
# even on a shared CI runner. Two measurements per side:
#   - criterion: every bench target in the src workspace (discovered with
#     `cargo metadata`, so new benches are picked up without editing this file),
#     saved as baselines `base` and `head` and compared with critcmp;
#   - instruction counts (Linux only, needs valgrind): `parser_cli decode` of the
#     Solana CLI fixture, render-only and with the intermediate output. Counts
#     barely move between runs, so they are the number to trust on noisy runners.
#
# The report goes to $GITHUB_STEP_SUMMARY when set, else stdout. Raw output is
# kept under $BENCH_OUT (default bench-out/).
set -euo pipefail

root=$(git rev-parse --show-toplevel)
# Accepts a SHA, a local ref, or a branch that only exists as origin/<name>.
resolve() {
  git -C "$root" rev-parse --verify -q "$1^{commit}" || git -C "$root" rev-parse --verify -q "origin/$1^{commit}"
}
base_ref=$(resolve "$1")
head_ref=$(resolve "$2")
filter=${3:-}
out=${BENCH_OUT:-$root/bench-out}
report=${GITHUB_STEP_SUMMARY:-/dev/stdout}
fixture=src/parser/cli/tests/fixtures/solana-json.input
mkdir -p "$out"

has_benches() {
  cargo metadata --no-deps --format-version 1 --manifest-path "$root/src/Cargo.toml" |
    jq -e '[.packages[].targets[] | select(.kind | index("bench"))] | length > 0' >/dev/null
}

run_criterion() {
  local side=$1
  local args=(--save-baseline "$side" --noplot)
  [ -n "$filter" ] && args+=("$filter")
  (cd "$root/src" && cargo bench --workspace --exclude parser_cli --bench '*' -- "${args[@]}") \
    >"$out/criterion-$side.log" 2>&1
}

# Prints the instruction count of one parser_cli run.
count_instructions() {
  local side=$1 mode=$2
  shift 2
  valgrind --tool=cachegrind --cache-sim=no --cachegrind-out-file=/dev/null \
    --log-file="$out/cachegrind-$side-$mode.log" "$root/src/target/release/parser_cli" "$@" >/dev/null 2>&1 ||
    { echo "parser_cli failed under valgrind ($side $mode)" >&2; return 1; }
  grep -oE 'I[[:space:]]+refs:[[:space:]]+[0-9,]+' "$out/cachegrind-$side-$mode.log" | grep -oE '[0-9,]+$' | tr -d ,
}

run_side() {
  local side=$1 ref=$2
  git -C "$root" checkout -q --detach "$ref"
  echo "== $side: $(git -C "$root" log --oneline -1)" >&2
  if has_benches; then
    run_criterion "$side"
    echo "$side" >>"$out/criterion-sides"
  fi
  if command -v valgrind >/dev/null && [ -f "$root/$fixture" ]; then
    (cd "$root/src" && cargo build --release -p parser_cli) >"$out/build-cli-$side.log" 2>&1
    mapfile -t cli_args <"$root/$fixture"
    # Standalone assignments so a failed run aborts instead of recording a count.
    local render intermediate
    render=$(cd "$out" && count_instructions "$side" render "${cli_args[@]}")
    intermediate=$(cd "$out" && count_instructions "$side" intermediate "${cli_args[@]}" --with-intermediate)
    printf '%s render %s\n%s intermediate %s\n' "$side" "$render" "$side" "$intermediate" >>"$out/instructions"
  fi
}

rm -rf "$root/src/target/criterion" "$out"/criterion-sides "$out"/instructions
start=$(git -C "$root" rev-parse HEAD)
trap 'git -C "$root" checkout -q --detach "$start"' EXIT
run_side base "$base_ref"
run_side head "$head_ref"

{
  echo "### $(uname -sm), $(nproc 2>/dev/null || sysctl -n hw.ncpu) CPUs"
  echo
  echo "base \`${base_ref:0:8}\`, head \`${head_ref:0:8}\`"
  echo
  if [ -f "$out/instructions" ]; then
    echo "**Instruction counts** (valgrind, \`parser_cli decode\` of \`$fixture\`):"
    echo
    echo "| mode | base | head | change |"
    echo "|---|---:|---:|---:|"
    for mode in render intermediate; do
      b=$(awk -v m="$mode" '$1 == "base" && $2 == m {print $3}' "$out/instructions")
      h=$(awk -v m="$mode" '$1 == "head" && $2 == m {print $3}' "$out/instructions")
      if [ -n "$b" ] && [ -n "$h" ]; then
        echo "| $mode | $b | $h | $(awk -v b="$b" -v h="$h" 'BEGIN {printf "%+.2f%%", (h - b) * 100 / b}') |"
      fi
    done
    echo
  fi
  if [ -f "$out/criterion-sides" ]; then
    echo "**Criterion** (wall clock; expect a few % of runner noise):"
    echo
    echo '```'
    if grep -qx base "$out/criterion-sides"; then
      (cd "$root/src" && critcmp base head)
    else
      echo "base has no bench targets; head only:"
      (cd "$root/src" && critcmp head)
    fi
    echo '```'
  else
    echo "No criterion bench targets on either side."
  fi
} >>"$report"
