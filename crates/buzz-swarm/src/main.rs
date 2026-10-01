//! Run Buzz agents on any configured relays from a Unix host.

#![deny(unsafe_code)]

#[cfg(unix)]
mod cli;
#[cfg(unix)]
mod config;
#[cfg(unix)]
mod env;
#[cfg(unix)]
mod identity;
#[cfg(unix)]
mod plan;
#[cfg(unix)]
mod supervisor;

#[cfg(unix)]
fn main() -> std::process::ExitCode {
    cli::main()
}

#[cfg(not(unix))]
fn main() -> std::process::ExitCode {
    eprintln!("buzz-swarm requires Unix process groups; run it on Linux or macOS");
    std::process::ExitCode::FAILURE
}
