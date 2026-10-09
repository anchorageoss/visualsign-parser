# Build recipe for the Anthropic OSS Scanner (https://red.anthropic.com/oss-scanner).
# The scanner builds this with the repository root as context, with network access; the audit that follows
# has no network, so everything (crates, git dependencies) is fetched here.
# Same digest-pinned Rust 1.88.0 stagex image that images/parser_app/Containerfile builds from.
FROM stagex/pallet-rust:1.88.0@sha256:b9021d2b75eac64fe8b931d96dde63ef11792e5023cee77c3471ccc34a95a377

ENV CARGO_PROFILE_DEV_DEBUG=2 \
    RUSTFLAGS="-C force-frame-pointers=yes"

# scanner contract: the checkout lives inside the image, at /src
COPY . /src
WORKDIR /src/src

RUN cargo fetch --locked

# Build every workspace target (integration tests need the binaries), with debug info and frame pointers.
# Mirrors `make build` / `make test` in src/Makefile (the image has no make); parser_cli is built apart from the
# rest so its dev-signing features do not unify into the production binaries. `codegen` is skipped: it is the
# protobuf generator (vendors protobuf-src, needs make) and its output in `generated` is checked in.
RUN cargo build --workspace --exclude parser_cli --exclude codegen --all-targets \
    && cargo build -p parser_cli --all-targets

# Run the test suite so the image is known to work, but do not abort the build on test failures.
RUN (cargo test --workspace --exclude parser_cli --exclude codegen --all-targets \
     && cargo test -p parser_cli --all-targets) || echo "WARNING: tests reported failures"
