---
name: solana-add-preset
description: Onboard a Solana program onto a shared preset family by writing a profile, mainnet fixtures and tests, or define a new family under core/ when no family fits. A family renders one lifecycle shared by many programs (today, receipt tokens -- vaults, backed tokens, liquid staking with a request/claim exit). Use when you have the program's IDL and sample transactions and its user actions match a catalogued lifecycle; fall back to solana-add-idl for a program that matches none.
user-invocable: true
---

# Add a Solana Preset on a Shared Family

## Scope

Three activities exist for Solana presets. This skill covers the last two.

1. **Bootstrap a generic IDL decoder**: `solana-add-idl`. The fallback for a program that matches no family.
2. **Onboard a program onto an existing family**: write a profile, fixtures and tests. Most new programs are this.
3. **Define a new family**: a shared renderer under `src/chain_parsers/visualsign-solana/src/core/<family>/`. Rare, and it needs design, so this skill only gives the conventions.

A family is a lifecycle many programs share, rendered once. A preset on a family declares only what differs between programs: program IDs, IDLs, account names, instruction names and wording. The renderer owns decoding, amount formatting, recipient verification, both layouts and the transaction summary, so every preset on the family behaves the same and inherits the same tests.

Do not regenerate or overwrite an existing preset. Presets are hand-owned once they land.

## Step 1: Gather inputs

Have all of these before writing code:

- **Program IDs.** One product may deploy several programs (Abra: a backed-token program and a liquid-staking program). One preset may cover all of them.
- **The IDL for each program**: its source, its `metadata.version`, and whether its `address` field is the deployed program ID or a placeholder.
- **One sample transaction per in-scope instruction**: signature, slot, the instruction's account order and its data. Prefer transactions signed after the program's latest upgrade.
- **The mints**, with decimals verified against `https://lite-api.jup.ag/tokens/v2/search?query=<mint>` (the token table's documented source) and the symbols the issuer uses.
- **The product's display rules**: the operation titles, and whether a row may name a leg the program settles at execution time ("Receive jlUSDC (estimated, rate at execution)").
- **The in-scope instruction list.** Everything else keeps the generic IDL view.

## Step 2: Pick the family

Apply the decision rule to the program's user actions, not its admin instructions:

- **Same family**: the lifecycle matches and only names differ (program, instruction and account names, verbs, titles). Write a profile.
- **New flow in an existing family**: one step differs (a two-step exit, a cancel) but the invariant the renderer verifies is unchanged. Add a `Flow` variant to the family, with unit tests there, then write the profile.
- **New family**: the verification rule or the row set differs structurally (a swap verifies a route and two legs, not a destination). See "Adding a new family". One program is a preset; two programs are a family.

### Catalog

#### Receipt token -- `core::receipt_token`

**Lifecycle.** The signer supplies an asset, receives a receipt token, and later redeems the receipt for the asset. Covers ERC-4626-style vaults (Jupiter Lend Earn), 1:1 backed tokens and exchange-rate liquid-staking tokens with a request/claim exit (Abra).

**Invariant verified.** The value lands with the signer: in the signer's associated token account for the received mint, or, for a request, in the escrow of the signer's own request account. The request account is re-derived from the IDL seeds and the escrow must be its associated token account. Any other destination is badged `THIRD PARTY`; anything that cannot be checked (an unresolved lookup-table account, an unknown token program) is badged `UNVERIFIED`. Only a verified destination lets the action name the transaction, and only when the signer is the fee payer.

**Flows** (`Flow`): what the exact amount argument denominates and which leg the program settles at its own rate.

| Flow | Exact amount | Counter leg | Bound |
|---|---|---|---|
| `Deposit` | asset in | receipt out | minimum |
| `Mint` | receipt out | asset in | maximum |
| `Withdraw` | asset out | receipt burned | maximum |
| `Redeem` | receipt burned | asset out | minimum |
| `RequestRedeem` | receipt set aside | asset at claim | minimum |
| `Claim { exact }` | asset or receipt, per program | asset out | minimum |
| `CancelRequest` | receipt returned | none | none |

**Profile** (`ReceiptTokenProgram`, one per program ID): `program_id`; `display_name` (the on-chain program name, used for the subtitle and the Program rows); `title_name` (the product name substituted for `{program}` in titles); the IDL account names `signer`, `asset_mint`, `receipt_mint` and `token_program`; `estimated_legs`; and `actions`. The profile carries no IDL: the renderer reads the IDL registered for `program_id` in `src/idl/builtin_programs.rs`.

**Action** (`Action::new(instruction, flow, label, title, amount_arg, recipient)` plus builders):

- `title` is a template; `{asset}`, `{receipt}` and `{program}` are substituted. It names tokens, never amounts: the Amount row carries the amount.
- `recipient` is `RecipientRule::SignerTokenAccount("<account>")`, or `RecipientRule::SignerEscrow { account, request, seeds, holds }` with `Seed::Literal(b"...")` / `Seed::Account("<account>")` and the denomination the escrow holds. `seeds` must include `Seed::Account(<signer>)`: an escrow keyed by anything else renders `UNVERIFIED` and never names the transaction.
- `.with_bound("<arg>")` for `*_with_min_*` / `*_with_max_*` variants; the bound's denomination and label follow the flow.
- `.with_full_position_title("...")` only when `u64::MAX` is verified on mainnet to mean the whole position (Jupiter plain `withdraw`). Everywhere else the literal renders flagged and claims nothing.
- `.with_asset_mint("<account>")` when the asset mint's account name differs per instruction.
- `.with_account_rows(&[("Paid to", "treasury_token_account")])` for program accounts the signer should see as address rows.

`estimated_legs: false` suppresses the "Receive / Pay / Burn ... (estimated, rate at execution)" rows. Bounds still render: they are instruction arguments, not estimates.

**Reference presets.** `presets/jupiter_earn`: one program, single-step exit, bounded variants, estimated legs, the `u64::MAX` sentinel. `presets/abra`: two programs in one preset, escrowed exits with seed derivation, no estimated legs, account rows.

## Step 3: Write the preset

Directory: `src/chain_parsers/visualsign-solana/src/presets/<snake_name>/`. `build.rs` registers `<PascalName>Visualizer` and every `.json` file in the directory; do not edit `presets/mod.rs` or any test file.

- **`<program>.json`**, one per program. If the issuer's IDL carries a placeholder `address`, replace it with the deployed program ID and say so in the module doc. Record the IDL source and version there too.
- **`mod.rs`**: program ID and display-name consts; the profiles as `const`/`static ReceiptTokenProgram`; a `static <NAME>_PROGRAMS: [ReceiptTokenProgram; N]`; `pub struct <PascalName>Visualizer` whose `InstructionVisualizer` impl delegates `visualize_tx_commands`, `transaction_summary` and `visualize_with_summary` to `receipt_token::visualize`, `receipt_token::transaction_summary` and `receipt_token::visualize_with_summary` (one decode serves the field and the summary), with `kind()` returning the `VisualizerKind` closest to the product; inline tests (each IDL loads, every discriminator is 8 bytes, `assert_profile_matches_idl` on every program, garbage and short data return errors).
- **`config.rs`**: the usual `SolanaIntegrationConfig` whose `data()` returns `receipt_token::config_data(&PROGRAMS)`.

Repository steps that are easy to miss:

- Register each IDL in `src/idl/builtin_programs.rs` (`preset_idl_configs` `ENTRIES`). The profile reads its IDL from there, and `preset_idls_are_registered` fails until every `.json` under `presets/` has an entry. A missing entry shows as `registered_source: Preset` with `parsed_instruction_data: None` in the intermediate output.
- Add every new mint to the token table in `src/utils/mod.rs`, with the verification date and source in the comment. Symbols come only from this table; an unknown mint renders raw units with a truncated address.
- Keep all wording ASCII and free of internal vocabulary. Use the field builders only. No `unwrap`, `expect` or `panic` outside tests.

## Step 4: Fixtures and tests

**Fixtures**: `tests/fixtures/<snake_name>/<name>.json`, one per in-scope instruction, in the shape of `tests/fixtures/jupiter_earn/deposit_usdc.json` (see `TESTING.md`). Use the instruction data and account list of the on-chain transaction, never data assembled from the IDL. `expected_fields` keys are labels normalized to lower case with underscores and are looked up across the condensed and expanded views, so an argument that shares its name with a row label (`amount`) cannot be asserted that way; use `raw_data` instead.

**Tests** live in `<snake_name>/tests/fixture_test.rs` and `tests/summary_test.rs`, declared from the inline `mod tests` (`mod fixture_test; mod summary_test;`). Use the shared helpers in `crate::test_utils` (`load_fixture`, `instruction_from_fixture`, `assert_fixture`, `visualize`, `visualize_context`, `condensed_field`, `condensed_value`, `find_value`, `title_of`, `legacy_transaction_b64`, `v0_transaction_b64`, `payload_for`, `top_level_value`, `top_level_labels`, `preview_titles`, `assert_no_summary`) and in `core::receipt_token::test_support` (`synthetic_instruction`, `assert_profile_matches_idl`).

Required coverage (copy the Abra tests and adapt):

- Every fixture: title and expected fields.
- One typed condensed view: Program as a named address, Amount as `amount_v2` with the symbol, Asset and Receipt token rows, the recipient row, the account rows.
- The recipient or escrow verified on the real sample; a swapped request account or escrow badged `THIRD PARTY`; an unresolved request account badged `UNVERIFIED`; an unresolved escrow or mint keeps the generic view.
- A forged `token_program` with the recipient or escrow derived under it badged `UNVERIFIED`; the same account under Token-2022 verified.
- An account row in a lookup table badged `UNVERIFIED`, with no summary.
- Fewer accounts than the IDL lists keeps the generic view.
- `u64::MAX` renders the flagged literal unless a sentinel is verified.
- An unknown mint renders raw units with no guessed symbol.
- Out-of-scope instructions keep the generic view.
- Summary: every operation signed by its fee payer proposes its title; the real transaction shape (nonce advance, compute budget, operation) keeps it; a third-party destination, a relayed transaction (fee payer is not the signer) and two actions in one transaction keep the default title.
- Intermediate output: `registered_source == Preset` and the instruction name decodes, for each program.

## Step 5: Verify

From `src/`:

```bash
make fmt          # plain `cargo fmt` skips the preset sources
make fmt-check
cargo clippy -p visualsign-solana --all-targets --locked -- -D warnings
cargo clippy -p visualsign-solana --features diagnostics --all-targets --locked -- -D warnings
cargo test -p visualsign-solana --locked
cargo test -p visualsign-solana --features diagnostics --locked
```

Then rebuild one sample transaction from its account keys and instruction data, and decode it end to end:

```bash
cargo run --bin parser_cli -- decode --chain solana --network SOLANA_MAINNET --output human -t <base64>
```

Check the title, the subtitle, the Amount row, the recipient or escrow row, and that the nonce and compute-budget instructions did not block the summary. After opening the PR, add the `test:surfpool` label for the live mainnet-fork fuzz job.

## Step 6: Record in the PR

- IDL source, version, and any `address` replacement.
- The sample transaction per instruction.
- Token verification source and date.
- The product decisions encoded in the profile: the titles and whether they are final, `estimated_legs`, and the instructions left out of scope.

## Adding a new family

Create a family when the second program with a shared lifecycle arrives, or when the first is a known ecosystem pattern.

1. Put the renderer in `src/core/<family>/mod.rs`. Family-specific test helpers go in a `test_support.rs` beside it, declared as `#[cfg(test)] #[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] pub(crate) mod test_support;`; generic helpers go in `crate::test_utils`. Declare the family in `src/core/mod.rs` as `pub(crate) mod <family>;` (never `pub`: `core` is re-exported from the crate root).
2. Define the profile as a struct of `&'static` fields so a preset can declare it as a `const` or `static`. Give the per-instruction type `const fn` builders with defaults.
3. Expose `visualize_with_summary(&[Profile], &VisualizerContext)`, which decodes once for the field and the summary, its `visualize` and `transaction_summary` projections, and `config_data(&[Profile])`. Resolve accounts through `InstructionView`, never `resolve_accounts()`.
4. Make the verification rule explicit and name its outcomes (verified, third party, unverifiable). Propose a summary only for a verified destination, and only when the signer is the fee payer.
5. Keep every program-specific word (titles, action labels, labels for program accounts) in the profile. Keep every rule (amount formatting, `u64::MAX`, unresolved placeholders, missing accounts) in the renderer, with unit tests there.
6. Land the family with two presets on it. Port an existing hand-written preset if it fits, and keep its output byte for byte; its tests are the proof.
7. Add a catalog entry to this skill: lifecycle, invariant, flows, profile fields, reference presets.
