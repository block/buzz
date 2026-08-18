use std::process::Command;

#[test]
fn desktop_identity_conflicts_with_environment_private_key() {
    let secret = "environment-secret-must-stay-redacted";
    let output = Command::new(env!("CARGO_BIN_EXE_buzz"))
        .env("BUZZ_PRIVATE_KEY", secret)
        .arg("--use-desktop-identity")
        .args(["channels", "list"])
        .output()
        .expect("run buzz CLI");

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.contains("cannot be used with"), "{stderr}");
    assert!(!stderr.contains(secret));
}

#[test]
fn missing_explicit_auth_does_not_discover_desktop_identity() {
    let output = Command::new(env!("CARGO_BIN_EXE_buzz"))
        .env_remove("BUZZ_PRIVATE_KEY")
        .args(["channels", "list"])
        .output()
        .expect("run buzz CLI");

    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.contains("BUZZ_PRIVATE_KEY is required"), "{stderr}");
    assert!(!stderr.contains("Desktop identity is missing"));
}
