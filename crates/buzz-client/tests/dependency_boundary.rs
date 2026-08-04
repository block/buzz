use std::path::Path;

const BANNED_DEPENDENCIES: &[&str] = &["buzz-cli", "tauri", "clap", "dotenv", "dirs"];
const BANNED_SOURCE_PATTERNS: &[&str] = &[
    "std::env::",
    "env::var(",
    "env::vars(",
    "std::io::stdin",
    "std::io::stdout",
    "std::io::stderr",
];

fn rust_sources(path: &Path, sources: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(path).expect("read source directory") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            rust_sources(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
}

#[test]
fn manifest_and_source_keep_the_application_boundary_out() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(crate_root.join("Cargo.toml")).expect("read manifest");
    for dependency in BANNED_DEPENDENCIES {
        assert!(
            !manifest.lines().any(|line| {
                let line = line.trim_start();
                !line.starts_with('#') && line.starts_with(dependency)
            }),
            "buzz-client must not depend on {dependency}"
        );
    }

    let mut sources = Vec::new();
    rust_sources(&crate_root.join("src"), &mut sources);
    for source in sources {
        let text = std::fs::read_to_string(&source).expect("read Rust source");
        for pattern in BANNED_SOURCE_PATTERNS {
            assert!(
                !text.contains(pattern),
                "{} contains application-only source pattern {pattern}",
                source.display()
            );
        }
    }
}
