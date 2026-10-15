#!/bin/bash -eu
# OSS-Fuzz build for visualsign-parser. google/oss-fuzz only holds a wrapper that runs this file, so
# the build can change here without a pull request there (the same split as .oss-scanner/).
# The nightly comes from the fuzz crate's rust-toolchain.toml, the single place the pin lives.
cd "$SRC/visualsign-parser/src/chain_parsers/visualsign-solana"
export RUSTUP_TOOLCHAIN
RUSTUP_TOOLCHAIN=$(sed -n 's/^channel = "\(nightly-[0-9-]*\)"/\1/p' fuzz/rust-toolchain.toml)
rustup toolchain install "$RUSTUP_TOOLCHAIN" --profile minimal

cargo fuzz build -O
FUZZ_TARGET_OUTPUT_DIR=fuzz/target/x86_64-unknown-linux-gnu/release
for f in fuzz/fuzz_targets/*.rs
do
    FUZZ_TARGET_NAME=$(basename "${f%.*}")
    cp "$FUZZ_TARGET_OUTPUT_DIR/$FUZZ_TARGET_NAME" "$OUT/"
done

# Seed the string-input target with a real transaction.
zip -j "$OUT/fuzz_transaction_string_seed_corpus.zip" ../../../testdata/solana_v0_alt.b64
