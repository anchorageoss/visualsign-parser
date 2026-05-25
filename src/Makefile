PARSER_HOST ?= 127.0.0.1
PARSER_PORT ?= 44020
PARSER_METRICS_PORT ?= 44021
PARSER_OUTER_SOCKET_PATH ?= /tmp/outer_parser.sock
PARSER_INNER_SOCKET_PATH ?= /tmp/inner_parser.sock


ROOT := $(shell git rev-parse --show-toplevel)

# Chains parser_app can be built with. Single source of truth for both
# narrow-build-check and feature-matrix-check below; keep in sync with
# the [features] block in parser_app/Cargo.toml. Listed alphabetically
# so the depth-2 pair generator in feature-matrix-check produces stable,
# unique pairs.
PARSER_APP_CHAINS := ethereum solana sui tron unspecified

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
	@# Wired into main CI; runs on every PR. Driven by
	@# $(PARSER_APP_CHAINS) above -- when a chain is added to
	@# parser_app/Cargo.toml, extend that variable and the matrix picks
	@# the new chain up.
	@#
	@# Includes clippy under each narrow feature set so dead-code /
	@# unused-import lints that only trigger when a chain is OFF are
	@# enforced (make lint above only covers default features).
	cargo build -p parser_app --no-default-features
	cargo clippy -p parser_app --no-default-features --all-targets -- -D warnings
	@set -e; for c in $(PARSER_APP_CHAINS); do \
		echo "==> features: $$c"; \
		cargo build -p parser_app --no-default-features --features $$c; \
		cargo clippy -p parser_app --no-default-features --features $$c --all-targets -- -D warnings; \
	done

.PHONY: feature-matrix-check
feature-matrix-check:
	@# Deep coverage of parser_app's feature gating: every chain alone,
	@# every chain + diagnostics, every depth-2 pair, and every depth-2
	@# pair + diagnostics. Hand-rolled (no cargo-hack dependency); matrix
	@# is generated from $(PARSER_APP_CHAINS) above.
	@#
	@# Note: `diagnostics` propagates to the always-on `visualsign` core
	@# crate, and weakly to `visualsign-solana` via the `?` syntax. The
	@# non-solana + diagnostics combos still exercise the weak-dep path
	@# (resolving to a no-op for visualsign-solana).
	@#
	@# LOCAL ONLY -- not wired into CI. The full matrix accumulates ~28GB
	@# in the cargo target dir; ensure ~30GB free in src/target before
	@# running. Use before releases or when touching feature definitions;
	@# the per-PR narrow-build-check above covers the common regression
	@# class on every change.
	@set -e; for c in $(PARSER_APP_CHAINS); do \
		echo "==> features: $$c"; \
		cargo build -p parser_app --no-default-features --features $$c; \
		echo "==> features: $$c diagnostics"; \
		cargo build -p parser_app --no-default-features --features "$$c diagnostics"; \
	done
	@set -e; i=0; for c1 in $(PARSER_APP_CHAINS); do \
		i=$$((i+1)); j=0; \
		for c2 in $(PARSER_APP_CHAINS); do \
			j=$$((j+1)); \
			if [ $$j -gt $$i ]; then \
				echo "==> features: $$c1 $$c2"; \
				cargo build -p parser_app --no-default-features --features "$$c1 $$c2"; \
				echo "==> features: $$c1 $$c2 diagnostics"; \
				cargo build -p parser_app --no-default-features --features "$$c1 $$c2 diagnostics"; \
			fi; \
		done; \
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
	@# The grpc-server binary refuses to start without EPHEMERAL_FILE (PRS-233).
	@# For local development we point it at the checked-in test fixture; do NOT
	@# reuse that fixture in any deployed/published environment.
	EPHEMERAL_FILE=$(ROOT)/src/integration/fixtures/ephemeral.secret \
		cargo run --bin parser_grpc_server

.PHONY: parser_gateway
parser_gateway:
	cargo run --bin parser_gateway
