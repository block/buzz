//! `buzz identity login|status|logout` run from the real binary without a key
//! or a valid auth tag; only `enroll-agent` needs them.

use std::process::{Command, Output};

fn buzz(home: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_buzz"))
        .args(args)
        // Keep the credential store inside the test's own directory.
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env_remove("BUZZ_PRIVATE_KEY")
        .env_remove("BUZZ_FI_ADAPTER")
        .env("BUZZ_AUTH_TAG", "[auth,stale]")
        .output()
        .unwrap()
}

#[test]
fn local_identity_commands_need_no_key_or_valid_auth_tag() {
    let home = tempfile::tempdir().unwrap();
    let text =
        |o: &Output| String::from_utf8_lossy(&[&o.stdout[..], &o.stderr[..]].concat()).into_owned();

    let status = buzz(home.path(), &["identity", "status"]);
    assert_eq!(status.status.code(), Some(0), "{}", text(&status));
    let json: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert!(json["session"].is_null());

    let logout = buzz(home.path(), &["identity", "logout"]);
    assert_eq!(logout.status.code(), Some(0), "{}", text(&logout));

    // Login gets as far as validating its own arguments.
    let login = buzz(
        home.path(),
        &["identity", "login", "--adapter", "http://adapter.example"],
    );
    assert_eq!(login.status.code(), Some(1), "{}", text(&login));
    assert!(text(&login).contains("--adapter"), "{}", text(&login));

    // Enrollment is the agent's own act: it still needs the key.
    let enroll = buzz(home.path(), &["identity", "enroll-agent"]);
    assert_eq!(enroll.status.code(), Some(3), "{}", text(&enroll));
    assert!(
        text(&enroll).contains("BUZZ_PRIVATE_KEY"),
        "{}",
        text(&enroll)
    );
}
