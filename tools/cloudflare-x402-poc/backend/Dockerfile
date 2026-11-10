FROM rust:1.88-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends cmake pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
ENV VERSION=poc
ADD src /src
WORKDIR /src
RUN cargo build --locked --release -p parser_grpc_server -p parser_gateway

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
# parser_grpc_server loads the checked-in dev ephemeral key relative to its cwd.
WORKDIR /app
COPY --from=build /src/target/release/parser_grpc_server /src/target/release/parser_gateway /usr/local/bin/
COPY --from=build /src/integration/fixtures /app/integration/fixtures
COPY entrypoint.sh /entrypoint.sh
ENTRYPOINT ["/entrypoint.sh"]
