// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Before anything else: WebKitGTK reads its rendering environment once at
    // process start, and this is the only point where the process is still
    // single threaded and no GTK object exists yet, which is what makes
    // `std::env::set_var` sound.
    #[cfg(target_os = "linux")]
    if let Err(diagnostic) = buzz_lib::webkit_rendering::apply() {
        eprintln!("buzz-desktop: {diagnostic}");
        std::process::exit(1);
    }

    buzz_lib::run()
}
