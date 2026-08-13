use std::process::Command;

#[test]
fn helper_subcommands_fail_owner_latch_before_spawning_an_agent() {
    for helper in ["models", "auth-methods", "authenticate"] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_buzz-acp"));
        command
            .arg(helper)
            .arg("--agent-command")
            .arg("/usr/bin/false")
            .env("BUZZ_ACP_REQUIRED_AGENT_OWNER", "a".repeat(64))
            .env_remove("BUZZ_ACP_AGENT_OWNER")
            .env_remove("BUZZ_AUTH_TAG")
            .env_remove("BUZZ_PRIVATE_KEY");
        if helper == "authenticate" {
            command.args(["--method-id", "test"]);
        }

        let output = command.output().expect("helper process must start");
        assert!(!output.status.success(), "{helper} must fail closed");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("required agent owner latch failed: no owner resolved"),
            "{helper} reached agent activity instead of the owner latch: {stderr}"
        );
        assert!(
            !stderr.contains("failed to spawn agent") && !stderr.contains("process exited"),
            "{helper} spawned the configured agent before enforcing the latch: {stderr}"
        );
    }

    let output = Command::new(env!("CARGO_BIN_EXE_buzz-acp"))
        .args(["models", "--agent-command", "/usr/bin/false"])
        .env("BUZZ_ACP_REQUIRED_AGENT_OWNER", "not-a-pubkey")
        .env("BUZZ_ACP_AGENT_OWNER", "not-a-pubkey")
        .env("BUZZ_PRIVATE_KEY", "01".repeat(32))
        .env_remove("BUZZ_AUTH_TAG")
        .output()
        .expect("invalid-equal helper process must start");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("must be exactly 64 lowercase hexadecimal"));
    assert!(!stderr.contains("failed to spawn agent"));
}
