use std::process::Command;

#[test]
fn dedicated_binary_refuses_other_providers_before_startup() {
    let output = Command::new(env!("CARGO_BIN_EXE_buzz-lmstudio-agent"))
        .env_clear()
        .env("BUZZ_AGENT_PROVIDER", "openai")
        .output()
        .expect("run dedicated binary");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("refuses provider"), "{stderr}");
}

#[test]
fn dedicated_binary_refuses_generic_auth_subcommand() {
    let output = Command::new(env!("CARGO_BIN_EXE_buzz-lmstudio-agent"))
        .arg("auth")
        .arg("databricks")
        .env_clear()
        .output()
        .expect("run dedicated binary");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not accept subcommands"), "{stderr}");
}
