fn main() {
    match buzz_lib::guardian_distribution::launcher_main() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("buzz-guardian-numbat: {error}");
            std::process::exit(1);
        }
    }
}
