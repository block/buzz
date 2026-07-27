use std::process::Command;

#[test]
fn help_hides_secret_environment_values() {
    let private_key_sentinel = "private-key-value-must-not-appear";
    let auth_tag_sentinel = "auth-tag-value-must-not-appear";

    let output = Command::new(env!("CARGO_BIN_EXE_buzz"))
        .arg("--help")
        .env("BUZZ_PRIVATE_KEY", private_key_sentinel)
        .env("BUZZ_AUTH_TAG", auth_tag_sentinel)
        .output()
        .expect("run buzz --help");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help output is UTF-8");

    assert!(stdout.contains("[env: BUZZ_PRIVATE_KEY]"));
    assert!(stdout.contains("[env: BUZZ_AUTH_TAG]"));
    assert!(!stdout.contains(private_key_sentinel));
    assert!(!stdout.contains(auth_tag_sentinel));
}
