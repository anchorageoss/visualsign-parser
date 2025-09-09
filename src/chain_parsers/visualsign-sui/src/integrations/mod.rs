//! DApp integrations live here.
//!
//! Each integration folder should follow the aligned structure:
//! - `mod.rs`: implements `<DappName>Visualizer` with `CommandVisualizer`
//! - `config.rs`: declares package, modules, functions, and typed indexers via `chain_config!`
//! - `aggregated_test_data.json`: test fixture consumed by `run_aggregated_fixture`
//!
//! See `src/presets/cetus`, `src/presets/suilend`, and `src/presets/momentum` for concrete examples.
//! For end-to-end steps, see the repository `CONTRIBUTING.md`.

