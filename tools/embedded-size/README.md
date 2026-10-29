# embedded-size (reference / PoC)

Measures how small a natively built parser can be for iOS and other embedded targets.
A minimal C-ABI wrapper (`vs_ethereum`, `vs_tron`, `vs_unspecified`, `vs_solana`, `vs_sui`,
`vs_near`, `vs_free`) exposes one entry point per chain, each behind a feature, so a build links
only the chains it calls.

This crate is standalone (its own `[workspace]`); it is not part of `src/`, so normal builds,
lint and tests are unaffected.

```sh
# one target, all rows (markdown table on stdout)
tools/embedded-size/measure.sh aarch64-apple-ios      # macOS + Xcode
tools/embedded-size/measure.sh x86_64-unknown-linux-gnu

# size-tuned build of a subset
cd tools/embedded-size && cargo build --profile embedded --no-default-features --features eth,tron
```

CI: `.github/workflows/embedded-size.yml` runs on demand or on PRs labeled `test:embedded-size`,
on macOS (iOS, darwin) and Linux (aarch64, x86_64), and writes the tables to the job summary.
