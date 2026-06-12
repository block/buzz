//! Integration tests for git-credential-nostr.
//!
//! Each test spawns the compiled binary as a subprocess, feeds it the
//! credential-helper protocol on stdin, and asserts on stdout/stderr/exit-code.

use std::io::Write;
use std::process::{Command, Stdio};

use base64::Engine as _;
use nostr::{Keys, ToBech32};

// ── helper ────────────────────────────────────────────────────────────────────

/// Spawn the binary, write `input` to stdin, collect output.
/// `env_vars` are added on top of the inherited environment.
/// `NOSTR_PRIVATE_KEY` is always cleared first to prevent test pollution.
fn run_helper(input: &str, env_vars: &[(&str, &str)]) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_git-credential-nostr");
    let mut cmd = Command::new(bin);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("NOSTR_PRIVATE_KEY")
        // Prevent git config on the test machine from supplying a keyfile.
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", std::env::temp_dir());
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
    child.wait_with_output().expect("failed to wait on child")
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

// ── tests ─────────────────────────────────────────────────────────────────────

/// Happy path: valid key + valid input → well-formed credential response with
/// a base64-encoded kind:27235 JSON event.
#[test]
fn happy_path() {
    let nsec = fresh_nsec();
    let out = run_helper(&valid_input(), &[("NOSTR_PRIVATE_KEY", &nsec)]);

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

/// Old git (no `capability[]=authtype` in input) → empty line on stdout, exit 0.
#[test]
fn old_git_no_authtype_capability() {
    let input = "protocol=https\n\
                 host=relay.example.com\n\
                 path=git/owner/repo.git/info/refs\n\
                 \n";

    let nsec = fresh_nsec();
    let out = run_helper(input, &[("NOSTR_PRIVATE_KEY", &nsec)]);

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

/// No key configured at all → exit 1, stderr mentions "no nostr key configured".
#[test]
fn missing_key() {
    // run_helper already clears NOSTR_PRIVATE_KEY and points HOME at a temp dir
    // that has no git config, so no keyfile will be found.
    let out = run_helper(&valid_input(), &[]);

    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1 for missing key"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no nostr key configured"),
        "expected 'no nostr key configured' in stderr, got:\n{stderr}"
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

    let nsec = fresh_nsec();
    let out = run_helper(input, &[("NOSTR_PRIVATE_KEY", &nsec)]);

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

    let nsec = fresh_nsec();
    let out = run_helper(input, &[("NOSTR_PRIVATE_KEY", &nsec)]);

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

/// Keyfile with 0644 permissions → exit 1, stderr mentions "insecure permissions".
#[cfg(unix)]
#[test]
fn bad_keyfile_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let nsec = fresh_nsec();

    // Write keyfile to a temp path.
    let tmp_dir = std::env::temp_dir();
    let keyfile = tmp_dir.join(format!(
        "nostr-test-key-{}.nsec",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::write(&keyfile, &nsec).expect("failed to write temp keyfile");

    // Set insecure permissions (0644).
    std::fs::set_permissions(&keyfile, std::fs::Permissions::from_mode(0o644))
        .expect("failed to set permissions");

    // Point a scratch git config at the keyfile.
    let git_config_dir = tmp_dir.join(format!(
        "nostr-test-gitconfig-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&git_config_dir).unwrap();
    let git_config_file = git_config_dir.join(".gitconfig");
    std::fs::write(
        &git_config_file,
        format!("[nostr]\n\tkeyfile = {}\n", keyfile.display()),
    )
    .expect("failed to write git config");

    let out = run_helper(
        &valid_input(),
        &[("HOME", git_config_dir.to_str().unwrap())],
    );

    // Clean up regardless of outcome.
    let _ = std::fs::remove_file(&keyfile);
    let _ = std::fs::remove_file(&git_config_file);
    let _ = std::fs::remove_dir(&git_config_dir);

    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1 for insecure keyfile permissions"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("insecure permissions"),
        "expected 'insecure permissions' in stderr, got:\n{stderr}"
    );
}
