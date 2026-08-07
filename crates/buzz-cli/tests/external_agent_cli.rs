use std::process::Command;

const TEST_PRIVATE_KEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const TEST_PUBLIC_KEY: &str = "4f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa";
const TEST_NPUB: &str = "npub1fu64hh9hes90w2808n8tjc2ajp5yhddjef0ctx4s7zmsgp6cwx4qgy4eg9";

#[test]
fn users_me_is_local_and_never_prints_private_key() {
    let output = Command::new(env!("CARGO_BIN_EXE_buzz"))
        .args(["users", "me"])
        .env("BUZZ_PRIVATE_KEY", TEST_PRIVATE_KEY)
        .env("BUZZ_RELAY_URL", "http://127.0.0.1:1")
        .env_remove("BUZZ_AUTH_TAG")
        .output()
        .expect("buzz users me should start");

    assert!(
        output.status.success(),
        "users me should not contact the unreachable relay: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "users me should not write stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8 JSON");
    assert!(!stdout.contains(TEST_PRIVATE_KEY));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(stdout.trim()).expect("valid identity JSON"),
        serde_json::json!({
            "pubkey": TEST_PUBLIC_KEY,
            "npub": TEST_NPUB,
        })
    );
}
