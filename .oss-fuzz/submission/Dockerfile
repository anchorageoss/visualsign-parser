FROM gcr.io/oss-fuzz-base/base-builder-rust
# openssl-sys (via solana-sdk) needs these to build.
RUN apt-get update && apt-get install -y pkg-config libssl-dev && apt-get clean && rm -rf /var/lib/apt/lists/*
RUN git clone --depth 1 https://github.com/anchorageoss/visualsign-parser visualsign-parser
WORKDIR $SRC/visualsign-parser
COPY build.sh $SRC/
