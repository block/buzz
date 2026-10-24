//! Integration tests for git-credential-nostr.
//!
//! Each test spawns the compiled binary as a subprocess, feeds it the
//! credential-helper protocol on stdin, and asserts on stdout/stderr/exit-code.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine as _;
use daz_secrets::BlockingClient;
use nostr::{Keys, ToBech32};

static NEXT_ACCOUNT: AtomicU64 = AtomicU64::new(1);

struct ProviderEntry {
    client: BlockingClient,
    service: String,
    account: String,
}

impl ProviderEntry {
    fn new(value: Option<&str>) -> Self {
        let client = BlockingClient::from_default_config().expect("daz-secrets test provider");
        let service = "buzz-credential-tests".to_string();
        let account = format!(
            "pid-{}-{}",
            std::process::id(),
            NEXT_ACCOUNT.fetch_add(1, Ordering::Relaxed)
        );
        if let Some(value) = value {
            client
                .set(&service, &account, value.as_bytes(), None)
                .expect("seed provider identity");
        }
        Self {
            client,
            service,
            account,
        }
    }
}

impl Drop for ProviderEntry {
    fn drop(&mut self) {
        let _ = self.client.delete(&self.service, &self.account, None);
    }
}

/// Spawn the binary against a real daz-secrets provider, write `input` to
/// stdin, and collect output. Secret bytes travel only through the provider.
fn run_helper(
    input: &str,
    provider_value: Option<&str>,
    env_vars: &[(&str, &str)],
) -> std::process::Output {
    let entry = ProviderEntry::new(provider_value);
    let bin = env!("CARGO_BIN_EXE_git-credential-nostr");
    let mut cmd = Command::new(bin);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(std::env::temp_dir())
        .env_remove("NOSTR_PRIVATE_KEY")
        .env_remove("BUZZ_AUTH_TAG")
        // Prevent git config on the test machine from supplying credentials.
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_COUNT", "2")
        .env("GIT_CONFIG_KEY_0", "nostr.secretService")
        .env("GIT_CONFIG_VALUE_0", &entry.service)
        .env("GIT_CONFIG_KEY_1", "nostr.secretAccount")
        .env("GIT_CONFIG_VALUE_1", &entry.account);
    for (k, v) in env_vars {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("failed to spawn git-credential-nostr");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().expect("failed to wait on child");
    drop(entry);
    output
}

/// Generate a fresh nsec string for use in tests.
fn fresh_nsec() -> String {
    let keys = Keys::generate();
    keys.secret_key().to_bech32().unwrap()
}

/// Standard valid credential-helper input (includes authtype capability).
fn valid_input() -> String {
    "capability[]=authtype\n\
     capability[]=state\n\
     protocol=https\n\
     host=relay.example.com\n\
     path=git/owner/repo.git/info/refs\n\
     wwwauth[]=Nostr realm=\"buzz\", method=\"GET\"\n\
     \n"
    .to_string()
}

/// Happy path: valid key + valid input → well-formed credential response with
/// a base64-encoded kind:27235 JSON event.
#[test]
fn happy_path() {
    let nsec = fresh_nsec();
    let out = run_helper(&valid_input(), Some(&nsec), &[]);

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();

    assert!(
        lines.contains(&"capability[]=authtype"),
        "missing capability[]=authtype in:\n{stdout}"
    );
    assert!(
        lines.contains(&"authtype=Nostr"),
        "missing authtype=Nostr in:\n{stdout}"
    );
    assert!(
        lines.contains(&"ephemeral=true"),
        "missing ephemeral=true in:\n{stdout}"
    );
    assert!(
        lines.contains(&"quit=true"),
        "missing quit=true in:\n{stdout}"
    );

    // Extract and validate the credential value.
    let cred_line = lines
        .iter()
        .find(|l| l.starts_with("credential="))
        .expect("no credential= line in output");
    let b64 = cred_line.strip_prefix("credential=").unwrap();

    let json_bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .expect("credential is not valid base64");
    let json_str = String::from_utf8(json_bytes).expect("credential is not valid UTF-8");

    let event: serde_json::Value =
        serde_json::from_str(&json_str).expect("credential does not decode to JSON");

    assert_eq!(
        event["kind"],
        serde_json::json!(27235),
        "expected kind 27235, got {}",
        event["kind"]
    );

    // Sanity-check a few more fields the NIP-98 event must have.
    assert!(event["id"].is_string(), "event missing 'id'");
    assert!(event["pubkey"].is_string(), "event missing 'pubkey'");
    assert!(event["sig"].is_string(), "event missing 'sig'");
    assert!(event["tags"].is_array(), "event missing 'tags'");
}

/// A Buzz-managed agent must carry its NIP-OA owner attestation inside the
/// signed NIP-98 event so the relay can admit it through the owner's membership.
#[test]
fn includes_nip_oa_auth_tag_in_signed_event() {
    let agent_keys = Keys::generate();
    let owner_keys = Keys::generate();
    let nsec = agent_keys.secret_key().to_bech32().unwrap();
    let auth_tag = serde_json::to_string(&[
        "auth",
        owner_keys.public_key().to_hex().as_str(),
        "",
        &"00".repeat(64),
    ])
    .expect("serialize auth tag");

    let out = run_helper(&valid_input(), Some(&nsec), &[("BUZZ_AUTH_TAG", &auth_tag)]);
    assert!(
        out.status.success(),
        "helper failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let credential = stdout
        .lines()
        .find_map(|line| line.strip_prefix("credential="))
        .expect("credential output");
    let event_json = base64::engine::general_purpose::STANDARD
        .decode(credential)
        .expect("base64 credential");
    let event: nostr::Event = serde_json::from_slice(&event_json).expect("NIP-98 event");

    assert!(
        event.verify().is_ok(),
        "auth tag must be covered by the event signature"
    );
    assert!(event.tags.iter().any(|tag| tag.as_slice()
        == [
            "auth",
            owner_keys.public_key().to_hex().as_str(),
            "",
            serde_json::from_str::<Vec<String>>(&auth_tag).unwrap()[3].as_str(),
        ]));
}

/// A configured but malformed owner attestation must fail closed rather than
/// silently authenticating the agent without delegation.
#[test]
fn malformed_nip_oa_auth_tag_fails_closed() {
    let nsec = fresh_nsec();
    let out = run_helper(
        &valid_input(),
        Some(&nsec),
        &[("BUZZ_AUTH_TAG", "not-json")],
    );

    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("invalid NIP-OA auth tag"));
    assert!(!String::from_utf8_lossy(&out.stdout).contains("credential="));
}

/// Old git (no `capability[]=authtype` in input) → empty line on stdout, exit 0.
#[test]
fn old_git_no_authtype_capability() {
    let input = "protocol=https\n\
                 host=relay.example.com\n\
                 path=git/owner/repo.git/info/refs\n\
                 \n";

    let out = run_helper(input, None, &[]);

    assert!(
        out.status.success(),
        "expected exit 0 for old-git path, got {:?}",
        out.status.code()
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    // Output should be just a blank line — no credential data.
    assert_eq!(
        stdout.trim(),
        "",
        "expected empty output for old-git path, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("credential="),
        "should not emit credential= for old git"
    );
}

/// No provider item configured → exit 1 without falling back to env or files.
#[test]
fn missing_key() {
    let out = run_helper(&valid_input(), None, &[]);

    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1 for missing key"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("identity is unavailable from daz-secrets"),
        "expected provider-unavailable identity error in stderr, got:\n{stderr}"
    );
}

/// `wwwauth[]` present but missing `method="..."` → exit 0, no credential emitted.
/// The helper gracefully declines rather than erroring, so git can fall through
/// to the next credential helper (safe for global credential.helper config).
#[test]
fn missing_method_hint() {
    let input = "capability[]=authtype\n\
                 capability[]=state\n\
                 protocol=https\n\
                 host=relay.example.com\n\
                 path=git/owner/repo.git/info/refs\n\
                 wwwauth[]=Nostr realm=\"buzz\"\n\
                 \n";

    let out = run_helper(input, None, &[]);

    assert!(
        out.status.success(),
        "expected exit 0 for missing method hint (graceful decline), got {:?}",
        out.status.code()
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("credential="),
        "should not emit credential= when method hint is missing"
    );
}

/// Input without `path=` line (useHttpPath not set) → exit 1, stderr mentions "useHttpPath".
/// The relay requires the full repo-root URL for NIP-98 verification, so the
/// credential helper cannot function without the path component.
#[test]
fn missing_path() {
    let input = "capability[]=authtype\n\
                 capability[]=state\n\
                 protocol=https\n\
                 host=relay.example.com\n\
                 wwwauth[]=Nostr realm=\"buzz\", method=\"GET\"\n\
                 \n";

    let out = run_helper(input, None, &[]);

    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1 for missing path"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("useHttpPath"),
        "expected 'useHttpPath' in stderr, got:\n{stderr}"
    );
}

/// Malformed provider bytes fail closed without emitting a credential.
#[test]
fn malformed_provider_key_fails_closed() {
    let out = run_helper(&valid_input(), Some("not-a-private-key"), &[]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("invalid nostr private key"),
        "got:\n{stderr}"
    );
    assert!(!String::from_utf8_lossy(&out.stdout).contains("credential="));
}
