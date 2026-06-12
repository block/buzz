fn main() {
    if let Err(e) = buzz_agent::run() {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
