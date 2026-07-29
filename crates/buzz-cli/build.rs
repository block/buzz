use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=BUZZ_GIT_REVISION");
    if std::env::var_os("BUZZ_GIT_REVISION").is_some() {
        return;
    }
    let manifest =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_else(|| ".".into()));
    let repository = manifest.join("../..");
    rerun_for_git_state(&repository);
    let revision = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(repository)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| {
            value.len() == 40
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        });
    if let Some(revision) = revision {
        println!("cargo:rustc-env=BUZZ_GIT_REVISION={revision}");
    }
}

fn rerun_for_git_state(repository: &std::path::Path) {
    let mut arguments = vec!["HEAD".to_string(), "packed-refs".to_string()];
    if let Some(symbolic) = Command::new("git")
        .args(["symbolic-ref", "-q", "HEAD"])
        .current_dir(repository)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        arguments.push(symbolic);
    }
    for argument in arguments {
        let path = Command::new("git")
            .args(["rev-parse", "--git-path", &argument])
            .current_dir(repository)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(path) = path {
            let path = PathBuf::from(path);
            let path = if path.is_absolute() {
                path
            } else {
                repository.join(path)
            };
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}
