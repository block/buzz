use std::process::Command;

#[test]
fn startup_and_failure_diagnostics_never_enter_acp_stdout() {
    let output = Command::new(env!("CARGO_BIN_EXE_buzz-acp"))
        .env("BUZZ_RELAY_URL", "ws://127.0.0.1:1")
        .env(
            "BUZZ_PRIVATE_KEY",
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .env("BUZZ_AGENT_COMMAND", "/definitely/missing/buzz-agent")
        .env("RUST_LOG", "buzz_acp=info")
        .output()
        .expect("buzz-acp process should start");

    assert!(
        !output.status.success(),
        "unreachable relay must fail startup"
    );
    assert!(
        output.stdout.is_empty(),
        "ACP stdout contained diagnostics: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("buzz-acp starting:"),
        "missing startup INFO: {stderr}"
    );
    assert!(
        stderr.contains("Error:") || stderr.contains("error"),
        "missing failure diagnostic: {stderr}"
    );
}
