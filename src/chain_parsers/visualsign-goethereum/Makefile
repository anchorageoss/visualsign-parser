# Makefile for visualsign-goethereum Ethereum transaction decoder

# Variables
BINARY_NAME = ethereum-decoder
GO_DIR = go
TARGET_DIR = target
BINARY_PATH = $(TARGET_DIR)/$(BINARY_NAME)

# Go build flags
GO_BUILD_FLAGS = -ldflags="-s -w"

# Default target
.PHONY: all
all: build

# Create target directory
$(TARGET_DIR):
	mkdir -p $(TARGET_DIR)

# Build the binary
.PHONY: build
build: $(TARGET_DIR)
	cd $(GO_DIR) && go build $(GO_BUILD_FLAGS) -o ../$(BINARY_PATH) main.go lib.go visualsign.go

# Clean build artifacts
.PHONY: clean
clean:
	rm -rf $(TARGET_DIR)
	cd $(GO_DIR) && go clean

# Download dependencies
.PHONY: deps
deps:
	cd $(GO_DIR) && go mod tidy && go mod download

# Generate test transaction and decode it
.PHONY: test-simple
test-simple:
	@echo "=== Testing simple ETH transfer ==="
	cd $(GO_DIR) && go test -v -run TestSimpleEthereumTransaction

# Test contract call with ABI
.PHONY: test-contract
test-contract:
	@echo "=== Testing smart contract call with ABI ==="
	cd $(GO_DIR) && go test -v -run TestERC20ContractTransaction

# Test the decoder functions
.PHONY: test-decoder
test-decoder:
	@echo "=== Testing decoder functions ==="
	cd $(GO_DIR) && go test -v -run TestDecoder

# Run all Go tests
.PHONY: test-go
test-go:
	@echo "=== Running all Go tests ==="
	cd $(GO_DIR) && go test -v ./...

# Test with the binary using a known transaction
.PHONY: test-binary
test-binary: build
	@echo "=== Testing binary with sample transaction ==="
	@echo "Testing with known transaction hex:"
	./$(BINARY_PATH) "0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83"

# Run all tests
.PHONY: test
test: test-go test-binary

# Check if binary exists
.PHONY: check
check:
	@if [ -f $(BINARY_PATH) ]; then \
		echo "✓ Binary exists: $(BINARY_PATH)"; \
		ls -la $(BINARY_PATH); \
	else \
		echo "✗ Binary not found: $(BINARY_PATH)"; \
		exit 1; \
	fi

# Show help
.PHONY: help
help:
	@echo "Available targets:"
	@echo "  build        - Build the ethereum-decoder binary"
	@echo "  clean        - Remove build artifacts"
	@echo "  deps         - Download Go dependencies"
	@echo "  test-simple  - Test simple ETH transfer decoding"
	@echo "  test-contract- Test smart contract call decoding"
	@echo "  test-decoder - Test decoder functions"
	@echo "  test-go      - Run all Go tests"
	@echo "  test-binary  - Test the built binary with a sample transaction"
	@echo "  test         - Run all tests"
	@echo "  check        - Check if binary exists"
	@echo "  help         - Show this help message"
	@echo ""
	@echo "Example usage:"
	@echo "  make build"
	@echo "  make test"
	@echo "  ./$(BINARY_PATH) 0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008026a0..."

# Install target (optional)
.PHONY: install
install: build
	@echo "Installing $(BINARY_NAME) to /usr/local/bin (requires sudo)"
	sudo cp $(BINARY_PATH) /usr/local/bin/$(BINARY_NAME)
	@echo "✓ Installed $(BINARY_NAME) to /usr/local/bin"
