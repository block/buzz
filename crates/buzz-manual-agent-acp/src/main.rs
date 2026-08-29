fn main() {
    if let Err(error) = buzz_manual_agent_acp::run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
