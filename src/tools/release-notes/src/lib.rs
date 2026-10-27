//! Draft a per-chain release announcement for visualsign-parser.
//!
//! Everything comes from git (tags, first-parent commits, files changed, co-author
//! trailers, the chain surface at each tag) and from public PR metadata via `gh`.
//! Nothing deployment-specific lives here: anything private, such as deploy details,
//! is passed in at run time with `--footer-file` and never stored in the repo.

pub mod chains;
pub mod entries;
pub mod mintlify;
pub mod render;
pub mod sources;
