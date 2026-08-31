//! Separate native tracer entry point; never starts the normal Desktop services.
fn main() {
    if let Err(error) = buzz_lib::run_host_start_tracer() {
        eprintln!("Start tracer failed: {error}");
        std::process::exit(1);
    }
}
