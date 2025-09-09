### Contributing to visualsign-sui

Thank you for helping expand VisualSign support on Sui. This guide explains how to add support for a new DApp/protocol integration to the visualsign-sui parser.

### Scope and structure

- **Where to add code**: Put new DApp-specific code under `src/integrations/<dapp_name>/`.
- **Reference examples**: See `src/presets/cetus/`, `src/presets/suilend/`, and `src/presets/momentum/`.
- **Aligned structure for each DApp module**:
  1. `mod.rs`: main parsing logic; implements `CommandVisualizer` and produces final VisualSign fields
  2. `config.rs`: declarative Sui package layout (package id, modules, functions, and argument indexers)
  3. `aggregated_test_data.json`: test fixture with base64-encoded tx blocks and field assertions

### Getting started

1) Create a folder: `src/integrations/<dapp_name>/` with files: `mod.rs`, `config.rs`, and `aggregated_test_data.json`.

2) Implement your visualizer in `mod.rs`:
   - Define `<DappName>Visualizer` and implement the `CommandVisualizer` trait:
     - `visualize_tx_commands(&self, context: &VisualizerContext)` must match only the package/module/functions your integration supports, using `get_config()` and `can_handle()` automatically provided by the trait.
     - Return a `Vec<AnnotatedPayloadField>` using `visualsign::field_builders` helpers like `create_text_field`, `create_amount_field`, `create_address_field`.
     - Set `kind()` to an appropriate `VisualizerKind` (e.g., `VisualizerKind::Dex("MyDex")`).

3) Declare your package in `config.rs` using the `chain_config!` macro:
   - Provide `package_id` and map `modules` → `functions`.
   - For each function, declare typed argument indexers so handlers in `mod.rs` can read values from `VisualizerContext` inputs.
   - See examples in `src/presets/*/config.rs`.

4) Add tests with `aggregated_test_data.json`:
   - Follow the format documented in `src/utils/test_helpers.rs`.
   - Each operation contains a base64-encoded `data` transaction and `asserts` for expected fields.
   - In your module tests (in `mod.rs`), call `include_str!("./aggregated_test_data.json")` and pass it to `run_aggregated_fixture`.

5) Build integration wiring: none required.
   - `build.rs` scans both `src/presets/` and `src/integrations/` and auto-registers any folder with a `mod.rs` exposing `<PascalCaseFolderName>Visualizer`.

### Technical guidelines

- Keep `mod.rs` focused on parsing and rendering by delegating offsets, indices, and decoding to the typed indexers from `config.rs`.
- Prefer typed accessors like `get_amount(context.inputs(), &pwc.arguments)` provided by the config index structs.
- Use `get_tx_type_arg(&pwc.type_arguments, idx)` to resolve type arguments into `SuiCoin`/`SuiPackage` where applicable.
- Produce concise, user-friendly titles and subtitles. Use `truncate_address` only where allowed in this codebase.

### Testing

- Unit tests should use `run_aggregated_fixture` from `src/utils/test_helpers.rs` to assert on the rendered fields.
- Add a `#[cfg(test)]` section in `mod.rs` to load `aggregated_test_data.json` and run the aggregated test.
- Ensure tests include enough variety to cover branches in `visualize_tx_commands` and helper handlers.

### Style and hygiene

- Follow Rust 2024 edition idioms and clippy warnings enabled in this crate.
- Keep function names descriptive and avoid unnecessary nesting.
- Return `VisualSignError::MissingData` for absent or unexpected inputs.
- Avoid unrelated refactors in PRs; keep changes scoped to the new integration.

### PR checklist

- [ ] `src/integrations/<dapp_name>/mod.rs` implements `CommandVisualizer`
- [ ] `src/integrations/<dapp_name>/config.rs` declares package, modules, functions, and indexers
- [ ] `src/integrations/<dapp_name>/aggregated_test_data.json` with multiple representative operations
- [ ] Tests in module run via `run_aggregated_fixture`
- [ ] `cargo build` and unit tests pass locally

If anything is unclear, check existing examples under `src/presets/` and open a discussion.


