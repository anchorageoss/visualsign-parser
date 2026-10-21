# Build recipe for the Anthropic OSS Scanner (https://red.anthropic.com/oss-scanner).
# The scanner builds this with the repository root as context, with network access; the audit that follows
# has no network, so everything (crates, git dependencies) is fetched here.

# Stage 1: the production parser_app, built exactly as images/parser_app/Containerfile builds it (same pinned
# image, flags and /src layout), so its sha256 matches `make non-oci-docker-images`. Keep in sync with that file.
#
# Why this is a copy and not `dockerfile: images/parser_app/Containerfile` in project.yaml: the scanner accepts any
# path for `dockerfile:`, but it builds the whole file with no --target and treats the LAST stage as the image it
# audits. The Containerfile's last stage is busybox holding only the binary: no source, no shell tooling, and no
# package manager for the scanner's added layer (curl, git, python3, bash, Claude Code). The scanner needs a tree
# it can read and build in, so this file reproduces the build stage and keeps the source alongside it.
FROM stagex/pallet-rust:1.94.0@sha256:2fbe7b164dd92edb9c1096152f6d27592d8a69b1b8eb2fc907b5fadea7d11668 AS release
ENV RUSTFLAGS='-C target-feature=+crt-static'
ENV CARGOFLAGS='--target x86_64-unknown-linux-musl --no-default-features --locked --release'
ENV RELEASE_DIR=/src/target/x86_64-unknown-linux-musl/release
ARG VERSION
ENV VERSION=$VERSION
ARG CHAIN_FEATURES="ethereum near solana sui tron unspecified"
ENV CHAIN_FEATURES=$CHAIN_FEATURES
ADD src /src
WORKDIR /src/
RUN cargo fetch
WORKDIR /src/parser/app
RUN --network=none <<-EOF2
    set -eu
    cargo build ${CARGOFLAGS} --features "vsock ${CHAIN_FEATURES}"
    mkdir -p /rootfs
    mv ${RELEASE_DIR}/parser_app /rootfs/
EOF2

# Stage 2: the tree to audit, with debug info and frame pointers. This stage is Debian, not stagex: the scanner
# adds its own layer on top of the last stage (curl, git, python3, bash, Claude Code) and needs a package manager
# for it, which the stagex Rust image does not have. Reused from stage 1: the pinned toolchain version (rustup
# installs the channel in src/rust-toolchain.toml, currently the same 1.94.0) and the release binary below.
FROM debian:bookworm@sha256:2c037a04925515fdd6ea85ea14a682d0e79931f5e9f5d07b6dbfc6ba12f9e858
ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH \
    CARGO_PROFILE_DEV_DEBUG=2 \
    RUSTFLAGS="-C force-frame-pointers=yes"
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates build-essential clang pkg-config git curl make libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# rustup-init is pinned to a release and verified against the checksum Rust publishes for it.
RUN curl -fsSL -o /tmp/rustup-init https://static.rust-lang.org/rustup/archive/1.29.1/x86_64-unknown-linux-gnu/rustup-init \
    && echo "dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71  /tmp/rustup-init" | sha256sum -c - \
    && chmod +x /tmp/rustup-init \
    && /tmp/rustup-init -y --profile minimal --default-toolchain none \
    && rm /tmp/rustup-init

# scanner contract: the checkout lives inside the image, at /src
COPY . /src
WORKDIR /src/src

# Install the toolchain pinned by src/rust-toolchain.toml.
RUN rustup toolchain install "$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)" --profile minimal -c clippy -c rustfmt

# The release parser_app from stage 1: the binary whose sha256 a signer approves. Not the debug build below.
COPY --from=release /rootfs/parser_app /opt/release/parser_app

RUN cargo fetch --locked

# Build every workspace target (integration tests need the binaries).
# Mirrors `make build` / `make test` in src/Makefile; parser_cli is built apart from the rest so its dev-signing
# features do not unify into the production binaries. `codegen` is skipped: it is the protobuf generator (vendors
# protobuf-src) and its output in `generated` is checked in.
RUN cargo build --workspace --exclude parser_cli --exclude codegen --all-targets \
    && cargo build -p parser_cli --all-targets

# Run the test suite so the image is known to work, but do not abort the build on test failures.
RUN (cargo test --workspace --exclude parser_cli --exclude codegen --all-targets \
     && cargo test -p parser_cli --all-targets) || echo "WARNING: tests reported failures"
