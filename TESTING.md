# Testing Guide for visualsign-parser

For chain-specific testing approaches, see:
- [Solana Testing Guide](src/chain_parsers/visualsign-solana/TESTING.md) - Fixture-based testing with real transaction data

## Test Coverage

### Prerequisites

```bash
# Install cargo-llvm-cov
cargo install cargo-llvm-cov
```

### Viewing Coverage

```bash
# Generate HTML report and open in browser (simplest approach)
cargo llvm-cov --workspace --open

# Or for a specific package
cargo llvm-cov --package visualsign-solana --lib --open
```

Reports are generated in `target/llvm-cov/html/` and opened automatically.

**For remote workstations:**

If you're developing on a remote machine and need to access the coverage report from your local browser:

```bash
# Generate HTML report (without --open)
cargo llvm-cov --package visualsign-solana --lib --html

# Serve the report on a port
cd target/llvm-cov/html
python3 -m http.server 8080

# Then access from your local machine at:
# http://<remote-host>:8080
```

Or use SSH port forwarding:
```bash
# On your local machine:
ssh -L 8080:localhost:8080 user@remote-host

# Then access at http://localhost:8080
```

### For CI/CD

```bash
# Generate lcov.info for external coverage tools (Codecov, Coveralls, etc.)
cargo llvm-cov --workspace --lcov --output-path lcov.info

# Or fail if coverage is below threshold
cargo llvm-cov --workspace --fail-under-lines 70
```

### More Information

See [cargo-llvm-cov documentation](https://github.com/taiki-e/cargo-llvm-cov) for advanced usage.

## Running Tests

```bash
# Run all tests
cargo test --workspace

# Run tests for a specific package
cargo test --package visualsign-solana

# Run with output visible
cargo test --package visualsign-solana -- --nocapture
```
