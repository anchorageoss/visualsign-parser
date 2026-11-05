# OSS-Fuzz

`submission/` holds the three files that go in `google/oss-fuzz` under `projects/visualsign-parser/`.
`build.sh` here is the real build; the upstream `build.sh` only execs it, so changes land through this
repository. The nightly comes from `src/chain_parsers/visualsign-solana/fuzz/rust-toolchain.toml`, which
`scripts/fuzz-nightly.sh` (weekly, `.github/workflows/fuzz-nightly.yml`) keeps current.

`submission/project.yaml` names `prasanna@anchorlabs.com` as `primary_contact`. OSS-Fuzz requires an established
committer's address (it must appear in the VCS logs) that is linked to a Google account. Try the files locally from a
`google/oss-fuzz` checkout before submitting:

    cp .oss-fuzz/submission/* <oss-fuzz>/projects/visualsign-parser/
    python3 infra/helper.py build_image visualsign-parser
    python3 infra/helper.py build_fuzzers --sanitizer address visualsign-parser
    python3 infra/helper.py check_build --sanitizer address visualsign-parser

## Known limitation: no coverage reports

`build.sh` overrides `RUSTUP_TOOLCHAIN` with our pinned nightly, because the base image's nightly (rustc 1.91)
can't build the crates: `generated` and `parser_cli_core` require rustc 1.94. With the override, fuzzing and the
coverage build both work. `helper.py coverage` then fails, though: base-runner's `llvm-profdata` reads profile
format 10, and our nightly's rustc writes format 11. Until OSS-Fuzz's base images catch up to rustc 1.94, the
project gets fuzzing and crash reports but no coverage reports. Keep the override; dropping it breaks the build.
