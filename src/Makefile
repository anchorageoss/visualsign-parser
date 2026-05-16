PARSER_HOST ?= 127.0.0.1
PARSER_PORT ?= 44020
PARSER_METRICS_PORT ?= 44021
PARSER_OUTER_SOCKET_PATH ?= /tmp/outer_parser.sock
PARSER_INNER_SOCKET_PATH ?= /tmp/inner_parser.sock


ROOT := $(shell git rev-parse --show-toplevel)

# Chains parser_app can be built with, derived from chain_parsers/visualsign-*.
# Adding a new chain crate under chain_parsers/visualsign-<name>/ automatically
# extends narrow-build-check below without further edits. Assumes the
# 1:1 convention: chain_parsers/visualsign-<name>/ <-> parser_app's
# `<name>` Cargo feature (see parser_app/Cargo.toml [features]).
PARSER_APP_CHAINS := $(patsubst visualsign-%,%,$(notdir $(wildcard chain_parsers/visualsign-*)))

.PHONY: all
all: generated
	make build

.PHONY: build
build:
	@# Split to avoid Cargo feature unification across the workspace.
	@# parser_cli enables visualsign-solana/diagnostics; building it together
	@# with parser_app/grpc-server would unify the feature ON for both.
	cargo build --workspace --exclude parser_cli
	cargo build -p parser_cli

.PHONY: test
test: build
	@# The integration tests rely on binaries from other crates being built, so
	@# we build all the workspace targets.
	make build
	@# diagnostics OFF: production payload shape (parser_app, integration, others).
	cargo test --workspace --exclude parser_cli --all-targets
	cargo test -p visualsign-solana --no-default-features --lib
	@# diagnostics ON: parser_cli + lib tests gated behind the feature.
	cargo test -p parser_cli --all-targets
	cargo test -p visualsign --features diagnostics --lib
	cargo test -p visualsign-solana --features diagnostics --lib

.PHONY: fmt
fmt:
	cargo fmt

.PHONY: lint
lint:
	cargo clippy --version
	cargo clippy --workspace --exclude parser_cli --all-targets -- -D warnings
	cargo clippy -p parser_cli --all-targets -- -D warnings
	cargo clippy -p visualsign --features diagnostics --lib -- -D warnings
	cargo clippy -p visualsign-solana --features diagnostics --lib -- -D warnings

.PHONY: narrow-build-check
narrow-build-check:
	@# Guards parser_app's per-chain feature gating. If a future change
	@# re-introduces an unconditional reference to a chain crate from
	@# parser_app, one of these narrow builds will fail to compile.
	@# Not wired into CI yet -- invoke manually or via a follow-up
	@# workflow. Driven by $(PARSER_APP_CHAINS) so new chain crates are
	@# picked up automatically.
	@#
	@# Includes clippy under each narrow feature set so dead-code /
	@# unused-import lints that only trigger when a chain is OFF are
	@# enforced (make lint above only covers default features).
	cargo build -p parser_app --no-default-features
	cargo clippy -p parser_app --no-default-features --all-targets -- -D warnings
	@set -e; for c in $(PARSER_APP_CHAINS); do \
		cargo build -p parser_app --no-default-features --features $$c; \
		cargo clippy -p parser_app --no-default-features --features $$c --all-targets -- -D warnings; \
	done

.PHONY: generated
generated:
	cargo run --manifest-path ./codegen/Cargo.toml && make fmt

.PHONY: parser
parser:
	make -j3 parser_host parser_enclave parser_app

.PHONY: parser_app
parser_app:
	cargo run --bin parser_app -- \
	--usock $(PARSER_INNER_SOCKET_PATH) \
	--ephemeral-file $(ROOT)/src/integration/fixtures/ephemeral.secret

.PHONY: parser_enclave
parser_enclave:
	cargo run --bin simulator_enclave $(PARSER_OUTER_SOCKET_PATH) $(PARSER_INNER_SOCKET_PATH)

.PHONY: parser_host
parser_host:
	cargo run --bin parser_host -- --host-ip $(PARSER_HOST) --host-port $(PARSER_PORT) --metrics --metrics-port $(PARSER_METRICS_PORT) --usock $(PARSER_OUTER_SOCKET_PATH)

.PHONY: grpc-server
grpc-server:
	cargo run --bin parser_grpc_server

.PHONY: parser_gateway
parser_gateway:
	cargo run --bin parser_gateway
