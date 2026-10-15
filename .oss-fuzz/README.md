# OSS-Fuzz

`submission/` holds the three files that go in `google/oss-fuzz` under `projects/visualsign-parser/`.
`build.sh` here is the real build; the upstream `build.sh` only execs it, so changes land through this
repository. The nightly comes from `src/chain_parsers/visualsign-solana/fuzz/rust-toolchain.toml`, which
`scripts/fuzz-nightly.sh` (weekly, `.github/workflows/fuzz-nightly.yml`) keeps current.

Before submitting, set a real `primary_contact` (an established committer's address linked to a Google
account) in `submission/project.yaml`, then try it locally from a `google/oss-fuzz` checkout:

    cp .oss-fuzz/submission/* <oss-fuzz>/projects/visualsign-parser/
    python3 infra/helper.py build_image visualsign-parser
    python3 infra/helper.py build_fuzzers --sanitizer address visualsign-parser
    python3 infra/helper.py check_build --sanitizer address visualsign-parser
