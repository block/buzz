#[cfg(unix)]
fn main() {
    std::process::exit(buzz_lib::run_operator_read_cli(std::env::args_os()));
}

#[cfg(not(unix))]
fn main() {
    eprintln!("buzz-read: this local control surface is available on Unix systems only");
    std::process::exit(1);
}
