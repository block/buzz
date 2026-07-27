fn main() {
    if let Err(error) = buzz_a2a_acp::run_cli() {
        eprintln!("buzz-a2a-acp: {error}");
        std::process::exit(2);
    }
}
