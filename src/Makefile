PARSER_HOST ?= 127.0.0.1
PARSER_PORT ?= 44020
PARSER_METRICS_PORT ?= 44021
PARSER_OUTER_SOCKET_PATH ?= /tmp/outer_parser.sock
PARSER_INNER_SOCKET_PATH ?= /tmp/inner_parser.sock


ROOT := $(shell git rev-parse --show-toplevel)

.PHONY: all
all:
	make build

.PHONY: build
build:
	cargo build --all

.PHONY: test
test: build
	@# The integration tests rely on binaries from other crates being built, so
	@# we build all the workspace targets.
	make build
	@# Run all tests
	cargo test --all-targets

.PHONY: fmt
fmt:
	cargo fmt

.PHONY: lint
lint:
	cargo clippy --version
	cargo clippy --all-targets -- -D warnings

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


out/parser_host/index.json: \
	$(shell git ls-files images/parser_host src) \
	out/common/index.json
	$(call build,parser_host)

out/parser_app/index.json: \
	$(shell git ls-files src/images/parser_app src) \
	out/common/index.json
	$(call build,parser_app)

out/common/index.json: \
	images/common/Containerfile
	$(call build,common)
