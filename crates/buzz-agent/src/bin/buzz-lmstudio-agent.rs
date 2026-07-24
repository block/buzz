fn main() {
    if let Err(error) = buzz_agent::run_lmstudio() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
