//! Binary entry point for the `buzz` CLI.
//!
//! Parses `std::env::args()` via [`buzz_cli::run_from_args`] and exits with the
//! returned process code.

#[tokio::main]
async fn main() {
    std::process::exit(buzz_cli::run_from_args(std::env::args()).await);
}
