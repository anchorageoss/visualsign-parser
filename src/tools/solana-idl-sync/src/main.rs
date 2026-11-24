//! Keep the Solana preset IDLs in sync with chain.
//!
//! See `src/lib.rs` for the layout; the real implementation is in the
//! library so tests can reach it.

fn main() {
    std::process::exit(solana_idl_sync::cli::main());
}
