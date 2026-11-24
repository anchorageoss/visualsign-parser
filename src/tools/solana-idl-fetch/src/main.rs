//! Fetch the Anchor IDL a Solana program publishes on chain.
//!
//! Usage:
//!   solana-idl-fetch <program_id> [--rpc URL] [--out FILE] [--compare FILE]
//!                    [--provenance FILE]
//!   solana-idl-fetch --record-repo <https-url> --commit <40-hex-sha>
//!                    --path <path-in-repo> --idl <idl.json> --program-id <id>
//!   solana-idl-fetch --record-issuer '<who supplied it>' [--notes '<text>']
//!                    --idl <idl.json> --program-id <id>
//!   solana-idl-fetch --record-unknown '<why the origin is unknown>'
//!                    --idl <idl.json> --program-id <id>
//!
//! See `src/lib.rs` and the module docs for the layout; the real
//! implementation is in the library so tests can reach it.

fn main() {
    std::process::exit(solana_idl_fetch::cli::main());
}
