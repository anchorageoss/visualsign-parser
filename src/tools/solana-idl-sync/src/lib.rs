//! Nightly sync of the Solana preset IDLs with the Anchor IDLs the programs
//! publish on chain.
//!
//! Driven by `.github/workflows/solana-idl-nightly.yml`:
//!
//! ```text
//! list-presets -> the matrix of programs + committed IDL paths to re-fetch
//! apply        -> write changed IDLs (and their provenance records) over the
//!                 committed files, and emit per-program compare sections
//! replay       -> decode recent mainnet transactions with parser_cli built
//!                 before and after the IDL change, one section per program
//! report       -> assemble report.md (PR-body sized) and the full report
//! ```
//!
//! Everything that talks to the network or runs a binary sits behind a
//! trait ([`replay::ReplayRpc`], [`replay::ParserRunner`]), so the tests in
//! this crate need neither network nor a built binary.

pub mod apply;
pub mod cli;
pub mod presets;
pub mod replay;
pub mod report;
pub mod sanitize;
