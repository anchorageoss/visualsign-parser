// TODO(#231): Remove these exemptions and fix violations in a follow-up PR.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

pub mod logger;

use parser_cli::cli::Cli;

fn main() {
    logger::setup_logger();

    Cli::execute()
}
