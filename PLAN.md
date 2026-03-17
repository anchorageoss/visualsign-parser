# Plan: Surfpool Integration with IDL-Fuzzed Transactions

## Overview

Run IDL-generated fuzzed instructions and real-transaction templates (fetched from RPC providers like Helius) through a surfpool mainnet fork. Validate that the parser produces correct, non-panicking output for all inputs.

The code lives in this repo (`visualsign-parser`). The [`visualsign-data-validation`](https://github.com/anchorageoss/visualsign-data-validation) e2e framework is used to pull reference data (real transactions, expected outputs) that seeds the template library here.

---

## Background

### What already exists

- **PropTest fuzz infrastructure** (`tests/fuzz_idl_parsing.rs`, `tests/pipeline_integration.rs`): generates random but structurally-valid IDL instructions and runs them through the parser. Tests are pure (no network, no RPC).
- **Fuzz script** (`scripts/fuzz_all_idls.sh`): runs PropTest fuzz cases against all 15 embedded IDLs.
- **Fixture system** (`tests/fixtures/jupiter_swap/`): JSON snapshots of real transactions with expected parser output. Used for regression testing.
- **PR #90 surfpool integration** (historical): introduced `solana_test_utils` crate with `SurfpoolManager`, `HeliusRpcProvider`, `NightlyTestRunner`, and a CI nightly workflow. That work is the direct predecessor of what we're building here.

### What we're adding

1. **Surfpool lifecycle management** re-introduced as a test utility crate.
2. **Helius (and optionally other RPC providers) transaction fetcher** to pull real mainnet transactions as fuzz templates.
3. **Template-seeded fuzzing**: derive PropTest strategies from real instruction shapes so fuzz cases are structurally plausible rather than fully random.
4. **Surfpool execution harness**: submit fuzzed instructions to a mainnet fork, capture simulation results, then run parser over them and assert safety/correctness.
5. **Nightly CI workflow** that runs the full end-to-end pipeline daily.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                         visualsign-parser                           │
│                                                                     │
│  ┌───────────────────────┐    ┌──────────────────────────────────┐  │
│  │  solana_test_utils    │    │    visualsign-solana (existing)  │  │
│  │  (new/restored crate) │    │                                  │  │
│  │                       │    │  - IdlRegistry                   │  │
│  │  SurfpoolManager      │    │  - transaction_to_visual_sign()  │  │
│  │  HeliusRpcProvider    │    │  - fuzz_idl_parsing tests        │  │
│  │  TemplateFuzzer       │────│  - pipeline_integration tests    │  │
│  │  NightlyTestRunner    │    └──────────────────────────────────┘  │
│  └───────────────────────┘                                          │
│           │                                                         │
│           │ fetch templates                                         │
│           ▼                                                         │
│  ┌────────────────┐    ┌────────────────────────────────────────┐   │
│  │  Helius API    │    │  visualsign-data-validation (external) │   │
│  │  (mainnet RPC) │    │  - reference outputs for seeding       │   │
│  └────────────────┘    │    fixture library                     │   │
│           │            └────────────────────────────────────────┘   │
│           │ mainnet fork                                            │
│           ▼                                                         │
│  ┌────────────────┐                                                 │
│  │  Surfpool      │                                                 │
│  │  (local fork)  │                                                 │
│  └────────────────┘                                                 │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Implementation Steps

### Step 1: Restore `solana_test_utils` crate

Create `src/solana_test_utils/` as a Rust library crate (dev/test only, not published).

**Files to create:**

```
src/solana_test_utils/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── surfpool/
    │   ├── mod.rs
    │   ├── manager.rs          # SurfpoolManager: spawn/kill, wait_ready, RPC client
    │   └── config.rs           # SurfpoolConfig: ports, fork URL, ledger path
    ├── rpc/
    │   ├── mod.rs
    │   ├── provider.rs         # RpcProvider trait
    │   ├── helius.rs           # HeliusRpcProvider
    │   └── mock.rs             # MockRpcProvider for unit tests
    ├── template/
    │   ├── mod.rs
    │   ├── fetcher.rs          # Fetch real txns and extract instruction templates
    │   ├── store.rs            # TemplateStore: on-disk JSON cache of templates
    │   └── fuzzer.rs           # TemplateFuzzer: PropTest strategies seeded from templates
    ├── runner/
    │   ├── mod.rs
    │   ├── harness.rs          # SurfpoolFuzzHarness: submit + parse + assert
    │   └── report.rs           # JSON + Markdown report generation
    └── common/
        ├── mod.rs
        └── programs.rs         # Well-known program IDs and trading pairs
```

**Key types:**

```rust
// surfpool/manager.rs
pub struct SurfpoolManager { process: Child, rpc_url: String, ws_url: String }
impl SurfpoolManager {
    pub async fn start(config: SurfpoolConfig) -> Result<Self>;
    pub async fn wait_ready(&self, timeout: Duration) -> Result<()>;
    pub fn rpc_url(&self) -> &str;
}
impl Drop for SurfpoolManager { /* kill process */ }

// rpc/provider.rs
#[async_trait]
pub trait RpcProvider: Send + Sync {
    async fn get_recent_transactions(
        &self, program_id: &Pubkey, limit: usize,
    ) -> Result<Vec<EncodedConfirmedTransactionWithStatusMeta>>;
}

// template/fetcher.rs
pub struct InstructionTemplate {
    pub program_id: Pubkey,
    pub idl_name: String,
    pub instruction_name: String,
    pub discriminator: [u8; 8],
    pub sample_data: Vec<u8>,          // real instruction data from mainnet
    pub account_count: usize,
}

// template/fuzzer.rs
pub struct TemplateFuzzer {
    templates: Vec<InstructionTemplate>,
}
impl TemplateFuzzer {
    /// PropTest strategy: keep discriminator, fuzz the argument bytes
    pub fn fuzz_args_strategy(&self) -> impl Strategy<Value = Instruction>;
    /// PropTest strategy: keep args, vary account list length/order
    pub fn fuzz_accounts_strategy(&self) -> impl Strategy<Value = Instruction>;
    /// PropTest strategy: fully random bytes after discriminator
    pub fn fuzz_chaos_strategy(&self) -> impl Strategy<Value = Instruction>;
}
```

---

### Step 2: Template library from Helius + data-validation

Fetch real transactions for each program in the embedded IDL set and store them as a template library.

**Programs to cover** (from existing IDL set):
- Jupiter Aggregator v6 (`JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4`)
- Orca Whirlpool (`whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc`)
- Drift Protocol (`dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH`)
- Meteora DLMM (`LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`)
- Raydium AMM (`675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8`)
- Kamino (`KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`)
- OpenBook (`opnb2LAfJYbRMAHHvqjCwQxanZn7n734bNwWKqChMQm`)

**Fetcher workflow:**
1. Call Helius `getSignaturesForAddress` for each program (last N signatures).
2. For each signature, call `getTransaction` to get full transaction data.
3. Extract instructions matching the program ID.
4. Identify the instruction name via discriminator lookup against the IDL.
5. Store as `InstructionTemplate` JSON in `src/solana_test_utils/templates/<idl_name>/`.

This template store is committed to the repo and refreshed by a separate CI step (not every test run). The `visualsign-data-validation` repo can also export template seeds here via a script or artifact.

---

### Step 3: Fuzz harness over surfpool

Create `src/solana_test_utils/tests/surfpool_fuzz.rs`:

```rust
/// Run fuzz variants of real instruction templates through surfpool.
/// Marked #[ignore] — run via: cargo test -p solana_test_utils -- --ignored
#[tokio::test]
#[ignore]
async fn fuzz_idl_instructions_through_surfpool() {
    let config = SurfpoolConfig {
        fork_url: std::env::var("HELIUS_RPC_URL").ok(),
        ..Default::default()
    };
    let surfpool = SurfpoolManager::start(config).await.unwrap();
    surfpool.wait_ready(Duration::from_secs(30)).await.unwrap();

    let templates = TemplateStore::load_all().unwrap();
    let fuzzer = TemplateFuzzer::new(templates);

    // Three fuzz modes per template
    proptest!(|(instr in fuzzer.fuzz_args_strategy())| {
        let result = simulate_and_parse(&surfpool, &instr);
        // Parser must never panic
        assert!(result.is_ok() || matches!(result, Err(ParseError::..)));
    });
}

async fn simulate_and_parse(
    surfpool: &SurfpoolManager,
    instr: &Instruction,
) -> Result<SignablePayload, ...> {
    // 1. Build minimal transaction around instruction
    // 2. simulateTransaction via surfpool RPC
    // 3. Pass transaction + metadata to transaction_to_visual_sign()
    // 4. Return parser result (ok or known error, never panic)
}
```

**Assertions:**
- Parser never panics (any `Err` result is acceptable).
- If discriminator matches a known IDL instruction, result must be `Ok(...)`.
- If `Ok`, the `SignablePayload` must be non-empty.
- No heap allocation blowup (guard via `SizeGuard` already in the fuzz tests).

---

### Step 4: Nightly CI workflow

Create `.github/workflows/nightly-surfpool.yml`:

```yaml
on:
  schedule:
    - cron: '0 3 * * *'   # 3 AM UTC daily
  workflow_dispatch:

jobs:
  surfpool-fuzz:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Install surfpool
        run: cargo install surfpool  # or pin to specific version
      - name: Run surfpool fuzz tests
        env:
          HELIUS_API_KEY: ${{ secrets.HELIUS_API_KEY }}
          HELIUS_RPC_URL: https://mainnet.helius-rpc.com/?api-key=${{ secrets.HELIUS_API_KEY }}
          PROPTEST_CASES: 512
        run: |
          cargo test -p solana_test_utils \
            --test surfpool_fuzz \
            -- --ignored --nocapture
      - name: Upload report
        uses: actions/upload-artifact@v4
        with:
          name: surfpool-fuzz-report-${{ github.run_id }}
          path: src/solana_test_utils/reports/
```

---

### Step 5: Multi-provider abstraction (Helius + others)

The `RpcProvider` trait makes it easy to add providers beyond Helius.

**Initial providers:**
- `HeliusRpcProvider`: uses Helius enhanced APIs (`getSignaturesForAddress`, `getTransaction`)
- `StandardRpcProvider`: plain Solana JSON-RPC (any endpoint, no API key required)
- `MockRpcProvider`: returns fixed template data, used in unit tests (no network)

**Provider selection** via env var or config:
```rust
pub fn rpc_provider_from_env() -> Box<dyn RpcProvider> {
    if let Ok(key) = std::env::var("HELIUS_API_KEY") {
        Box::new(HeliusRpcProvider::new(key))
    } else if let Ok(url) = std::env::var("SOLANA_RPC_URL") {
        Box::new(StandardRpcProvider::new(url))
    } else {
        Box::new(MockRpcProvider::default())
    }
}
```

---

## Relationship with `visualsign-data-validation`

`visualsign-data-validation` is an external e2e framework used to:
- Pull reference transactions from mainnet with known-good expected outputs.
- Export those as JSON fixtures consumable by `solana_test_utils/templates/`.

**Workflow:**
1. Run the data-validation pipeline to generate template seeds (separate repo, separate run).
2. Commit the resulting template JSON files into `src/solana_test_utils/templates/` in *this* repo.
3. The fuzz tests here read from those committed templates — no live network call needed during test execution (only during template refresh).

This keeps the fuzz test runs fast and hermetic while still being grounded in real mainnet instruction shapes.

---

## File Summary

| Path | Purpose |
|------|---------|
| `src/solana_test_utils/` | New crate: surfpool + RPC + template + fuzzer |
| `src/solana_test_utils/templates/<idl>/` | Committed template JSON from Helius |
| `src/solana_test_utils/tests/surfpool_fuzz.rs` | Main fuzz-over-surfpool test |
| `src/solana_test_utils/tests/template_fetcher.rs` | Tests for template fetching (mocked) |
| `.github/workflows/nightly-surfpool.yml` | Daily CI fuzz run |
| `scripts/refresh_templates.sh` | One-off script to re-seed template library |

---

## Open Questions

1. **Surfpool version**: What version is compatible with the current Solana SDK (`2.1.15` / `2.2.7`)? PR #90 noted compatibility friction; pin explicitly in `Cargo.toml`.
2. **Account setup in surfpool**: Simulating instructions that reference specific accounts (e.g., pool vaults) requires those accounts to exist in the fork. Does surfpool auto-clone accounts from mainnet, or do we need explicit `--clone-account` flags?
3. **Template refresh cadence**: How often should `scripts/refresh_templates.sh` be run and committed? Weekly seems reasonable.
4. **data-validation export format**: What JSON schema should `visualsign-data-validation` export so templates are directly importable by `TemplateStore::load_all()`?
5. **Fuzz depth per IDL**: Some IDLs (Drift: 199 instructions) are large. Should we limit coverage to the top N instructions by usage frequency?
