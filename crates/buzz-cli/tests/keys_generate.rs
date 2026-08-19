//! Integration tests for `buzz keys generate`.
//!
//! These spawn the compiled binary as a subprocess because the two contracts
//! that matter most cannot be observed from a unit test:
//!
//! 1. **Local-only.** The command must run with no `BUZZ_PRIVATE_KEY`, no
//!    `BUZZ_RELAY_URL`, and no network. Unit tests call `cmd_generate` directly
//!    and so bypass `run()`, which is exactly where the "private key is
//!    required" gate lives — the gate this command has to be dispatched before.
//! 2. **The secret stays off stdout.** Only a real invocation proves what the
//!    process actually wrote to its stdout stream.

use std::process::Command;

/// Spawn `buzz` with a scrubbed environment: no identity, no relay, no auth
/// tag. Any of those leaking in from the developer's shell would mask a
/// regression where the command started depending on them.
fn run_keys(args: &[&str]) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_buzz");
    Command::new(bin)
        .args(args)
        .current_dir(std::env::temp_dir())
        .env_remove("BUZZ_PRIVATE_KEY")
        .env_remove("BUZZ_RELAY_URL")
        .env_remove("BUZZ_AUTH_TAG")
        .output()
        .expect("failed to spawn buzz")
}

fn parse_stdout(out: &std::process::Output) -> serde_json::Value {
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout was not JSON ({e}): {stdout}"))
}

#[test]
fn generates_without_a_private_key_or_relay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.nsec");

    let out = run_keys(&["keys", "generate", "--out", path.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "expected success, got {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );

    let report = parse_stdout(&out);
    let pubkey = report["pubkey"].as_str().expect("pubkey in report");
    assert_eq!(pubkey.len(), 64, "pubkey should be 64-char hex: {pubkey}");
    assert!(pubkey.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(report["npub"]
        .as_str()
        .expect("npub in report")
        .starts_with("npub1"));

    // The written secret is a real nsec, and it is the one the reported pubkey
    // belongs to — the round trip a caller depends on.
    let nsec = std::fs::read_to_string(&path).unwrap();
    let nsec = nsec.trim();
    assert!(nsec.starts_with("nsec1"), "expected an nsec, got: {nsec}");
    let reloaded = nostr::Keys::parse(nsec).expect("generated nsec must parse");
    assert_eq!(reloaded.public_key().to_hex(), pubkey);
}

#[test]
fn does_not_print_the_secret_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.nsec");

    let out = run_keys(&["keys", "generate", "--out", path.to_str().unwrap()]);
    assert!(out.status.success());

    let nsec = std::fs::read_to_string(&path).unwrap();
    let nsec = nsec.trim();

    // Neither the literal secret nor the bech32 prefix may appear on either
    // stream. Checking both streams matters: an accidental `eprintln!` of the
    // key is just as much of a leak as a `println!`.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stdout.contains(nsec), "secret key leaked to stdout");
    assert!(!stderr.contains(nsec), "secret key leaked to stderr");
    assert!(!stdout.contains("nsec1"), "nsec prefix appeared on stdout");
    assert!(!stderr.contains("nsec1"), "nsec prefix appeared on stderr");

    let report = parse_stdout(&out);
    assert!(
        report.get("nsec").is_none(),
        "report must not carry the secret without --stdout"
    );
}

#[test]
fn stdout_flag_is_an_explicit_opt_in() {
    let out = run_keys(&["keys", "generate", "--stdout"]);
    assert!(out.status.success());

    let report = parse_stdout(&out);
    let nsec = report["nsec"]
        .as_str()
        .expect("nsec in report with --stdout");
    assert!(nsec.starts_with("nsec1"));

    // With no --out there is no file, so no path is reported.
    assert!(report.get("secret_key_path").is_none());

    let reloaded = nostr::Keys::parse(nsec).expect("piped nsec must parse");
    assert_eq!(
        reloaded.public_key().to_hex(),
        report["pubkey"].as_str().unwrap()
    );
}

#[test]
fn refuses_a_destinationless_invocation() {
    let out = run_keys(&["keys", "generate"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected input-error exit code 1; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty(), "no key should be reported");
}

#[test]
fn refuses_to_clobber_an_existing_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.nsec");
    let arg = path.to_str().unwrap();

    let first = run_keys(&["keys", "generate", "--out", arg]);
    assert!(first.status.success());
    let original = std::fs::read_to_string(&path).unwrap();

    let second = run_keys(&["keys", "generate", "--out", arg]);
    assert_eq!(second.status.code(), Some(1), "expected refusal");

    // The live identity is intact — this is the property that protects an
    // already-connected agent from a re-run of the connect flow.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);

    let forced = run_keys(&["keys", "generate", "--out", arg, "--force"]);
    assert!(forced.status.success(), "--force should replace the file");
    assert_ne!(std::fs::read_to_string(&path).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn written_secret_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.nsec");

    let out = run_keys(&["keys", "generate", "--out", path.to_str().unwrap()]);
    assert!(out.status.success());

    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o777,
        0o600,
        "secret key file must be owner read/write only, got {:o}",
        mode & 0o777
    );
}
