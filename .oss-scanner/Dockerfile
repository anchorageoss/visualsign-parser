# Build recipe for the Anthropic OSS Scanner (https://red.anthropic.com/oss-scanner).
# The scanner builds this with the repository root as context, with network access; the audit that follows
# has no network, so everything (toolchain, protoc, crates, git dependencies) is fetched here.
FROM debian:bookworm
ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH \
    CARGO_PROFILE_DEV_DEBUG=2 \
    RUSTFLAGS="-C force-frame-pointers=yes"
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates build-essential clang pkg-config git curl unzip make libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# protoc v21.4, the version CI pins
RUN curl -fsSL -o /tmp/protoc.zip \
      https://github.com/protocolbuffers/protobuf/releases/download/v21.4/protoc-21.4-linux-x86_64.zip \
    && unzip -q /tmp/protoc.zip -d /usr/local bin/protoc 'include/*' \
    && rm /tmp/protoc.zip

# scanner contract: the checkout lives inside the image, at /src
COPY . /src
WORKDIR /src/src

# Install the toolchain pinned by src/rust-toolchain.toml, then fetch every crate.
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none \
    && rustup toolchain install "$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)" \
         --profile minimal -c clippy -c rustfmt
RUN cargo fetch --locked

# Build every workspace target (integration tests need the binaries), with debug info and frame pointers.
RUN make build

# Run the test suite so the image is known to work, but do not abort the build on test failures.
RUN (make test || echo "WARNING: make test reported failures")
