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
