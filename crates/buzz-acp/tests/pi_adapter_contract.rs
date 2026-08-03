use std::path::PathBuf;

#[tokio::test]
async fn real_pi_adapter_persists_and_resets_one_logical_thread() {
    let Some(adapter_entrypoint) = std::env::var_os("BUZZ_PI_AGENT_BIN").map(PathBuf::from) else {
        eprintln!(
            "skipping real Pi adapter contract smoke; set BUZZ_PI_AGENT_BIN or run `just pi-agent-contract-test`"
        );
        return;
    };
    let adapter_entrypoint = adapter_entrypoint
        .canonicalize()
        .expect("BUZZ_PI_AGENT_BIN must point to the built dist/cli.js");
    let cwd = std::env::current_dir()
        .expect("read test cwd")
        .canonicalize()
        .expect("canonicalize test cwd");
    let root = std::env::temp_dir().join(format!(
        "buzz-pi-adapter-contract-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let state_dir = root.join("state");
    let pi_agent_dir = root.join("pi-agent");

    let result =
        buzz_acp::verify_pi_adapter_contract(&adapter_entrypoint, &state_dir, &pi_agent_dir, &cwd)
            .await;
    if let Err(error) = std::fs::remove_dir_all(&root) {
        eprintln!("warning: could not remove contract-smoke state: {error}");
    }
    result.expect("real Buzz ACP ↔ Pi adapter contract must hold");
}
