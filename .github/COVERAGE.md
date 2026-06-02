# Code Coverage CI Guide

This document explains how the code coverage CI system works for the VisualSign Parser project.

## Overview

The coverage CI system provides:
- **Per-package coverage** analysis using `cargo-llvm-cov`
- **Dependency-aware testing** - automatically tests packages that depend on changed code
- **Coverage diff reports** - shows how your PR affects coverage
- **Automated PR comments** - coverage report posted directly on pull requests
- **Threshold enforcement** - prevents merging code that drops coverage significantly

## How It Works

### 1. Change Detection

When you open a PR, the CI system:
1. Detects which Rust files changed
2. Identifies which packages contain those files
3. Finds all packages that depend on the changed packages
4. Creates a test matrix for all affected packages

**Example:**
```
Changed file: src/visualsign/src/lib.rs
↓
Detected package: visualsign
↓
Found dependents: visualsign-ethereum, visualsign-solana, visualsign-sui, parser_app
↓
Will test: visualsign + all 4 dependents
```

### 2. Coverage Generation

For each affected package:
1. Runs tests with instrumentation (`cargo llvm-cov`)
2. Generates LCOV format coverage data
3. Parses coverage statistics (lines, coverage %)
4. Saves as artifacts for comparison

### 3. Coverage Comparison

The CI system:
1. Downloads coverage from the base branch (from previous runs)
2. Compares current PR coverage with base
3. Calculates diff for each package
4. Determines if changes meet thresholds

### 4. PR Comment

A markdown report is posted to your PR showing:
- Coverage % for each affected package
- Change from base branch (📈 +2.5% or 📉 -1.2%)
- Status indicators (✅ Excellent, 🟢 Good, 🟡 Needs Work, 🔴 Critical)
- Detailed module-level breakdown
- Threshold violations (if any)

## Example PR Comment

```markdown
# 📊 Code Coverage Report

This PR affects the following packages. Coverage changes are shown below.

## 📦 Package Coverage

| Package | Coverage | Change | Status |
|---------|----------|--------|--------|
| **visualsign** | 84.65% | ➡️ 0.00% | ✅ Excellent |
| **visualsign-solana** | 75.23% | 📈 +2.07% | 🟢 Good |
| **parser_app** | 45.50% | 📉 -3.22% | 🔴 Critical |

## 📈 Summary

⚠️ **Warning:** Some packages have coverage below acceptable thresholds.
```

## Configuration

Coverage thresholds are defined in `.github/coverage-config.yml`:

```yaml
packages:
  visualsign:
    critical: 80
    warning: 85
    good: 90
    max_drop: 3.0
```

### Threshold Levels

| Level | Meaning | Action |
|-------|---------|--------|
| **Critical** | < 50% | ❌ Fails CI check |
| **Warning** | 50-69% | ⚠️ Generates warning |
| **Good** | 70-79% | 🟢 Acceptable |
| **Excellent** | ≥ 80% | ✅ Meets standards |

### Maximum Drop

If coverage for any package drops more than `max_drop` percentage points, the CI check fails.

Example: `visualsign` has `max_drop: 3.0`, so:
- Base: 85% → PR: 82% = ✅ OK (drop of 3%)
- Base: 85% → PR: 81% = ❌ FAIL (drop of 4%)

## Viewing Coverage Locally

Run the same coverage analysis locally:

```bash
# For a single package
cd src
cargo llvm-cov --package visualsign --lcov --output-path coverage.lcov

# View HTML report
cargo llvm-cov --package visualsign --html
open target/llvm-cov/html/index.html
```

## Testing Dependents Locally

Find which packages depend on your changes:

```bash
# From repo root
.github/scripts/detect-changed-packages.sh
```

## Coverage Goals by Package Type

### Core Libraries (visualsign)
- **Target:** ≥ 80%
- **Why:** Core library affects all parsers, must be reliable

### Chain Parsers (ethereum, solana, sui)
- **Target:** ≥ 70%
- **Why:** User-facing functionality, important for correctness

### DApp Integrations (presets)
- **Target:** ≥ 80%
- **Why:** Protocol-specific logic needs thorough testing

### Service Layer (parser_app)
- **Target:** ≥ 60%
- **Why:** Integration with external systems, harder to test

### Stub Implementations (tron, unspecified)
- **Target:** ≥ 50%
- **Why:** Not yet fully implemented

## Improving Coverage

### 1. Identify Gaps

Check the PR comment for packages with low coverage:

```
| **visualsign-solana** | 73.16% | Status: 🟢 Good |
  - preset:stakepool: 11.6% 🔴 CRITICAL GAP
  - preset:system: 42.2% 🟡 Needs Work
```

### 2. Add Tests

Focus on untested areas:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stakepool_deposit() {
        // Add test for untested function
    }
}
```

### 3. Run Coverage Locally

```bash
cargo llvm-cov --package visualsign-solana --html
open target/llvm-cov/html/index.html
```

The HTML report highlights which lines are not covered.

### 4. Check Impact

```bash
# See coverage before
cargo llvm-cov --package visualsign-solana

# Add tests...

# See coverage after
cargo llvm-cov --package visualsign-solana
```

## Skipping Coverage

In rare cases, you may want to skip coverage CI:

```
[skip coverage]
```

Add this to your commit message to skip the coverage workflow.

**Note:** Only use this for:
- Documentation-only changes
- CI configuration changes
- Non-code changes

## Troubleshooting

### Coverage not generated

**Symptom:** CI shows "No coverage file found"

**Causes:**
- No tests exist for the package
- Tests failed to compile
- Tests crashed during execution

**Solution:**
1. Check CI logs for test failures
2. Run tests locally: `cargo test --package <name>`
3. Add tests if none exist

### Coverage tool error (Sui)

**Symptom:** `error: failed to collect object files`

**Cause:** Some packages (especially Sui) have compatibility issues with `cargo-llvm-cov`

**Workaround:**
- Tests still run and verify correctness
- Coverage metrics just aren't collected
- Being investigated (see issue #XX)

### Base coverage not available

**Symptom:** "Base branch coverage data not available"

**Cause:** First time running coverage on a branch

**Solution:**
- After first merge to main, future PRs will have base coverage
- The system learns from the first run

### CI check failing

**Symptom:** "Coverage Status Check" fails

**Reasons:**
1. Package below critical threshold (< 50%)
2. Coverage dropped > 5% from base
3. New code added without tests

**Solution:**
1. Add tests to increase coverage
2. Update thresholds in `.github/coverage-config.yml` (if justified)
3. Ask for review if you believe threshold is too strict

## Best Practices

### 1. Write Tests Early

Add tests as you write code, don't wait until PR:

```rust
// Write implementation
pub fn parse_transaction(data: &[u8]) -> Result<Tx> {
    // ...
}

// Immediately add test
#[cfg(test)]
mod tests {
    #[test]
    fn test_parse_transaction_valid() { ... }

    #[test]
    fn test_parse_transaction_invalid() { ... }
}
```

### 2. Test Error Cases

Coverage includes error paths:

```rust
#[test]
fn test_parse_transaction_invalid_format() {
    let result = parse_transaction(&[]);
    assert!(result.is_err());
}
```

### 3. Test Edge Cases

Don't just test happy paths:

```rust
#[test]
fn test_empty_input() { ... }

#[test]
fn test_maximum_size() { ... }

#[test]
fn test_boundary_values() { ... }
```

### 4. Use Test Fixtures

For complex data:

```rust
#[test]
fn test_jupiter_swap() {
    let tx = include_str!("../fixtures/jupiter-swap.txt");
    let result = parse(tx);
    assert!(result.is_ok());
}
```

### 5. Monitor Dependent Impact

When changing core libraries, check dependent coverage:
- Did your change break tests in other packages?
- Did you add new functionality that dependents should test?

## FAQ

### Q: Why test dependents when I only changed one package?

**A:** If you change `visualsign` (core library), all parsers depend on it. We test dependents to ensure your change didn't break them or reduce their coverage.

### Q: Can I disable coverage CI?

**A:** Not recommended, but you can skip individual runs with `[skip coverage]` in commit message.

### Q: What if I can't reach the threshold?

**A:**
1. Add more tests (preferred)
2. Discuss with team if threshold should be lowered
3. Document why coverage is difficult (e.g., external dependencies)

### Q: How often is base coverage updated?

**A:** Every time a PR is merged to main. The next PR will compare against the updated base.

### Q: Does coverage affect merge?

**A:** Yes, if a package drops below critical threshold or drops > 5%, the CI check fails.

## Scripts Reference

| Script | Purpose |
|--------|---------|
| `detect-changed-packages.sh` | Finds affected packages |
| `parse-coverage.sh` | Extracts stats from LCOV |
| `generate-coverage-diff.sh` | Creates PR comment |
| `check-coverage-thresholds.sh` | Enforces thresholds |
| `get-base-coverage.sh` | Downloads base branch coverage |

## GitHub Actions Workflow

The main workflow: `.github/workflows/coverage.yml`

**Jobs:**
1. `detect-changes` - Find affected packages
2. `coverage` - Generate coverage for each package (matrix)
3. `coverage-report` - Compare and post PR comment
4. `coverage-status` - Enforce thresholds

**Triggers:**
- Pull requests to `main`
- Pushes to `main` (updates base coverage)
- Paths: `src/**/*.rs`, `src/**/Cargo.toml`

## Support

For questions or issues with coverage CI:
1. Check this document
2. Check CI logs for specific errors
3. Open an issue with `ci:coverage` label
4. Ask in #engineering channel

---

**Last Updated:** 2025-11-17
**Maintained by:** Engineering Team
