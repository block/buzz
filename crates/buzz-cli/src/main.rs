mod terminal;

#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    let code = if terminal::requested(&args) {
        terminal::run(args)
    } else {
        buzz_cli::run_from_args(args).await
    };
    std::process::exit(code);
}
