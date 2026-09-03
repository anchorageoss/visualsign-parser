PARSER_HOST ?= 127.0.0.1
PARSER_PORT ?= 44020
PARSER_METRICS_PORT ?= 44021
PARSER_OUTER_SOCKET_PATH ?= /tmp/outer_parser.sock
PARSER_INNER_SOCKET_PATH ?= /tmp/inner_parser.sock


ROOT := $(shell git rev-parse --show-toplevel)

# Chains parser_app can be built with -- derived from the
# `chain_parsers/visualsign-<name>/` directory layout that the workspace
# already uses as its source of truth. `$(sort ...)` deduplicates and
# alphabetizes so the depth-2 pair generator in feature-matrix-check
# produces stable, unique pairs across machines (filesystem-listing
# order isn't guaranteed otherwise).
#
# Convention: every `chain_parsers/visualsign-<name>/` directory has a
# matching `<name>` feature in parser_app/Cargo.toml. A chain crate added
# without that feature wired in fails loudly at narrow-build-check on
# the PR that introduces it -- which is the right place to catch it.
PARSER_APP_CHAINS := $(sort $(patsubst visualsign-%,%,$(notdir $(wildcard chain_parsers/visualsign-*))))

# Chains parser_cli can be built with. Unlike PARSER_APP_CHAINS this is not
# derived from chain_parsers/* -- parser_cli only wires up chains that have a
# ChainPlugin registered (see parser/cli/src/), which sui and unspecified
# don't yet, so a subset is the actual rule and a literal list says so.
PARSER_CLI_CHAINS := ethereum near solana tron

.PHONY: all
all: generated
	make build

.PHONY: build
build:
	@# Split to avoid Cargo feature unification across the workspace.
	@# parser_cli enables visualsign-solana/diagnostics,
	@# visualsign-ethereum/dev-signing, and visualsign-near/dev-signing;
	@# building it together with parser_app/grpc-server would unify those
	@# features ON for both, putting the CLI dev signing keys (and the
	@# allowlist entries that trust them) into the production binaries.
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
	cargo test -p parser_cli_core --features diagnostics --lib
	cargo test -p visualsign-near --features diagnostics --lib
	cargo test -p visualsign-solana --features diagnostics --lib

# Solana preset sources, which `cargo fmt` cannot reach. `presets/mod.rs`
# declares its submodules through `include!(concat!(env!("OUT_DIR"), ...))`, a
# path that exists only after codegen, and rustfmt does not follow an `include!`
# of a generated path -- so `cargo fmt` formats `presets/mod.rs` and nothing
# beneath it. These files are enumerated so `fmt` covers them directly.
#
# `find` rather than a literal list: presets are scaffolded one directory per
# program and the set grows with each new one, so a hardcoded list drops files
# out of the gate as soon as it goes stale.
SOLANA_PRESET_SOURCES := $(shell find chain_parsers/visualsign-solana/src/presets -name '*.rs' 2>/dev/null | sort)

.PHONY: fmt
fmt:
	cargo fmt
	@test -n "$(SOLANA_PRESET_SOURCES)" || { echo "no preset sources found under chain_parsers/visualsign-solana/src/presets"; exit 1; }
	@echo "rustfmt $(words $(SOLANA_PRESET_SOURCES)) Solana preset sources"
	@rustfmt $(SOLANA_PRESET_SOURCES)

.PHONY: lint
lint:
	cargo clippy --version
	cargo clippy --workspace --exclude parser_cli --all-targets -- -D warnings
	cargo clippy -p parser_cli --all-targets -- -D warnings
	cargo clippy -p visualsign --features diagnostics --lib -- -D warnings
	cargo clippy -p parser_cli_core --features diagnostics --lib -- -D warnings
	cargo clippy -p visualsign-near --features diagnostics --lib -- -D warnings
	cargo clippy -p visualsign-solana --features diagnostics --lib -- -D warnings

.PHONY: narrow-build-check
narrow-build-check:
	@# Guards parser_app's per-chain feature gating. If a future change
	@# re-introduces an unconditional reference to a chain crate from
	@# parser_app, one of these narrow checks will fail to compile.
	@# Wired into main CI; runs on every PR. Driven by
	@# $(PARSER_APP_CHAINS) above, which auto-extends when a new
	@# chain_parsers/visualsign-<name>/ directory is added.
	@#
	@# Uses `cargo check` rather than `cargo build`: the failure modes
	@# we want to catch here are compile-level (unconditional crate
	@# references that should have been cfg-gated). `check` produces no
	@# linker output, runs ~2x faster, and matches what clippy needs.
	@#
	@# Clippy runs under each narrow feature set so dead-code /
	@# unused-import lints that only trigger when a chain is OFF are
	@# enforced (make lint above only covers default features).
	@#
	@# Note: parser_app re-type-checks under each distinct feature set,
	@# but workspace dependencies are reused from the cache that earlier
	@# CI steps (make test / make lint) already populated -- so the
	@# marginal CI cost is dominated by parser_app's own check time.
	cargo check -p parser_app --no-default-features
	cargo clippy -p parser_app --no-default-features --all-targets -- -D warnings
	@set -e; for c in $(PARSER_APP_CHAINS); do \
		echo "==> features: $$c"; \
		cargo check -p parser_app --no-default-features --features $$c; \
		cargo clippy -p parser_app --no-default-features --features $$c --all-targets -- -D warnings; \
	done
	make dev-signing-absent-check

.PHONY: parser-cli-narrow-build-check
parser-cli-narrow-build-check:
	@# parser_cli's own release artifact (images/parser_cli/Containerfile)
	@# builds it with only one chain feature enabled (e.g. --features
	@# solana) -- a configuration `make test` never exercises, since that
	@# always builds and tests parser_cli with every default feature on.
	@# A dependency left unconditional in a chain crate, but gated only
	@# behind that crate's own default features, compiles fine there and
	@# breaks silently in this narrower build until someone builds the
	@# release image (e.g. bs58 in visualsign-solana).
	@#
	@# `cargo test`, not `check`: the failure mode of interest is the
	@# compiled binary panicking or misbehaving against real fixtures
	@# (tests/cli_test.rs runs CARGO_BIN_EXE_parser_cli end to end), not
	@# just a compile error.
	@# `--locked`: matches the release container's flags and keeps this
	@# check from silently updating Cargo.lock instead of failing.
	@set -e; for c in $(PARSER_CLI_CHAINS); do \
		echo "==> parser_cli --features $$c"; \
		cargo test -p parser_cli --locked --no-default-features --features $$c --all-targets; \
	done

.PHONY: dev-signing-absent-check
dev-signing-absent-check:
	@# `dev-signing` carries hardcoded signing-key seeds and allowlists them,
	@# so it must never resolve in a binary that ships. Only parser_cli
	@# enables it (`ethereum`/`near` features above pull
	@# visualsign-{ethereum,near}/dev-signing), and the ONLY thing keeping it
	@# off parser_app/grpc-server is that `build` compiles parser_cli in a
	@# separate cargo invocation -- Cargo unifies features across a single
	@# invocation, so one `cargo build --workspace` covering both would turn
	@# dev-signing ON for the attested binary.
	@#
	@# Asserted against the recipes rather than the resolved feature graph:
	@# `cargo tree -e features` does not print enabled features that carry no
	@# dependency edges, and `dev-signing = []` is exactly that, so a graph
	@# probe silently matches nothing and passes regardless.
	@set -e; \
	bad=$$(grep -nE '^[[:space:]]*cargo (build|test|check|clippy).*--workspace' Makefile \
		| grep -v 'exclude parser_cli' || true); \
	if [ -n "$$bad" ]; then \
		echo "ERROR: --workspace recipe without '--exclude parser_cli' would unify"; \
		echo "       dev-signing onto parser_app/grpc-server:"; \
		echo "$$bad"; \
		exit 1; \
	fi; \
	echo "==> no --workspace recipe pulls parser_cli into a shared feature resolution"

.PHONY: feature-matrix-check
feature-matrix-check:
	@# Deep coverage of parser_app's feature gating: diagnostics alone
	@# (no chains), every chain alone, every chain + diagnostics, every
	@# depth-2 pair, and every depth-2 pair + diagnostics. Hand-rolled
	@# (no cargo-hack dependency); matrix is generated from
	@# $(PARSER_APP_CHAINS) above.
	@#
	@# Note: `diagnostics` propagates to the always-on `visualsign` core
	@# crate, and weakly to `visualsign-near`/`visualsign-solana` via the
	@# `?` syntax. Combos without those chains still exercise the weak-dep
	@# path (resolving to a no-op for the absent crate).
	@#
	@# Uses `cargo check` for the same reason as narrow-build-check
	@# above -- the goal is compile-level regression detection, not
	@# producing binaries.
	@#
	@# LOCAL ONLY -- not wired into CI. The full matrix grows the cargo
	@# target dir by a few GB. Run before releases or when touching
	@# feature definitions; the per-PR narrow-build-check above covers
	@# the common regression class on every change.
	echo "==> features: diagnostics"
	cargo check -p parser_app --no-default-features --features diagnostics
	@set -e; for c in $(PARSER_APP_CHAINS); do \
		echo "==> features: $$c"; \
		cargo check -p parser_app --no-default-features --features $$c; \
		echo "==> features: $$c diagnostics"; \
		cargo check -p parser_app --no-default-features --features "$$c diagnostics"; \
	done
	@set -e; i=0; for c1 in $(PARSER_APP_CHAINS); do \
		i=$$((i+1)); j=0; \
		for c2 in $(PARSER_APP_CHAINS); do \
			j=$$((j+1)); \
			if [ $$j -gt $$i ]; then \
				echo "==> features: $$c1 $$c2"; \
				cargo check -p parser_app --no-default-features --features "$$c1 $$c2"; \
				echo "==> features: $$c1 $$c2 diagnostics"; \
				cargo check -p parser_app --no-default-features --features "$$c1 $$c2 diagnostics"; \
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
	cargo run --bin parser_grpc_server

.PHONY: parser_gateway
parser_gateway:
	cargo run --bin parser_gateway
