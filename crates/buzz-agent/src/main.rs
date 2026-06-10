fn main() {
    if let Err(e) = sprout_agent::run() {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
