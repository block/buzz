// Prevents any additional console window on Windows, DO NOT REMOVE!!
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "windows")]
    if buzz_lib::managed_launcher::run_if_requested() {
        return;
    }

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
