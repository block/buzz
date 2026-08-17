#[tokio::main]
async fn main() {
    std::process::exit(buzz_cli::run_from_args(std::env::args()).await);
}
