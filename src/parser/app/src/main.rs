// TODO(#231): Remove these exemptions and fix violations in a follow-up PR.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use parser_app::cli::Cli;

#[tokio::main]
async fn main() {
    Cli::execute().await
}
