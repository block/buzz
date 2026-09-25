//! Run the installed harness/helper personalities across a real process boundary.
#![cfg(unix)]

use base64::Engine;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn harness_native_git_and_startup_shutdown_cleanup() {
    check_native_git_and_startup_shutdown_cleanup(false);
}

#[test]
fn task_native_git_and_startup_shutdown_cleanup() {
    check_native_git_and_startup_shutdown_cleanup(true);
}

fn check_native_git_and_startup_shutdown_cleanup(task: bool) {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path();
    let adapter = workspace.join("adapter");
    // Nothing in this script configures Git: all settings/helpers must come
    // from the production harness spawn, including a CLI-only identity.
    std::fs::write(&adapter, r#"#!/bin/sh
set -eu
cd "$PROBE_DIR"
test -z "${NOSTR_PRIVATE_KEY:-}"
test "$(git config test.inherited)" = preserved
test "$(git config user.name)" = 'Git Probe'
test "$(git config --get-urlmatch credential.helper https://relay.invalid/git/test/repo)" = nostr
! git config --get-urlmatch credential.helper https://unrelated.invalid/git/test/repo
! git config --get-urlmatch credential.helper https://relay.invalid/other/repo
! git config --get-urlmatch credential.helper https://relay.invalid/git-other/repo
git config nostr.keyfile > keyfile
command -v git-sign-nostr > signer
command -v git-credential-nostr > credential
git init -q repo
cd repo
git -c core.hooksPath=/dev/null commit -s --allow-empty -qm 'bootstrap probe'
git tag -m 'signed tag probe' probe
git verify-commit HEAD
git verify-tag probe
git show -s --format='%an%n%ae%n%cn%n%ce' HEAD > ../identity
printf 'capability[]=authtype\nprotocol=https\nhost=relay.invalid\npath=git/test/repo\nwwwauth[]=Nostr method="GET"\n\n' | git credential fill > ../credential-result
printf '%s' "${BUZZ_PRIVATE_KEY}" > ../buzz_private_key
printf '%s' "${BUZZ_RELAY_URL}" > ../buzz_relay_url
printf 'done' > ../done
# Stay alive until the harness is terminated during ACP initialization.
exec sleep 60
"#).unwrap();
    std::fs::set_permissions(&adapter, std::fs::Permissions::from_mode(0o700)).unwrap();
    let keys = nostr::Keys::generate();
    let log = std::fs::File::create(workspace.join("harness.log")).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_buzz-acp"));
    if task {
        let path = workspace.join("task.json");
        std::fs::write(
            &path,
            serde_json::json!({
                "version": 1, "taskId": "git-probe", "agentPubkey": keys.public_key().to_hex(),
                "prompt": "probe", "maxDurationMs": 30000
            })
            .to_string(),
        )
        .unwrap();
        command
            .arg("run")
            .arg("--task")
            .arg(path)
            .arg("--no-memory");
    }
    let mut child = command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .args([
            "--private-key",
            &keys.secret_key().to_secret_hex(),
            "--relay-url",
            "wss://relay.invalid",
            "--agent-command",
            adapter.to_str().unwrap(),
            "--agent-args",
            "",
        ])
        .env("PROBE_DIR", workspace)
        .env("BUZZ_ACP_DISPLAY_NAME", "Git Probe")
        .env("GIT_AUTHOR_NAME", "Inherited Human")
        .env("GIT_AUTHOR_EMAIL", "human@example.invalid")
        .env("GIT_COMMITTER_NAME", "Inherited Human")
        .env("GIT_COMMITTER_EMAIL", "human@example.invalid")
        .env("NOSTR_PRIVATE_KEY", "must-not-reach-adapter")
        .env("GIT_CONFIG_COUNT", "3")
        .env("GIT_CONFIG_KEY_0", "test.inherited")
        .env("GIT_CONFIG_VALUE_0", "preserved")
        // Emulate an old Desktop launcher's scoped entries. These must be
        // carried through and overridden without duplicating config counts.
        .env(
            "GIT_CONFIG_KEY_1",
            "credential.https://relay.invalid/git.helper",
        )
        .env("GIT_CONFIG_VALUE_1", "/nonexistent/old-helper")
        .env(
            "GIT_CONFIG_KEY_2",
            "credential.https://relay.invalid/git.useHttpPath",
        )
        .env("GIT_CONFIG_VALUE_2", "true")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_PARAMETERS", "'user.name=Inherited Human'")
        .env_remove("BUZZ_ACP_SETUP_PAYLOAD")
        .env_remove("BUZZ_AUTH_TAG")
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !workspace.join("done").exists() && Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // Always terminate, even on assertion failure, so regressions don't leave
    // the subprocess or its ephemeral signing key behind.
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(child.id() as i32),
        nix::sys::signal::Signal::SIGTERM,
    )
    .ok();
    let exit_deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() && Instant::now() < exit_deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
    }
    let status = child.wait().unwrap();
    let logs = std::fs::read_to_string(workspace.join("harness.log")).unwrap();
    assert!(workspace.join("done").exists(), "probe failed: {logs}");
    assert_eq!(
        status.code(),
        Some(if task { 143 } else { 0 }),
        "startup shutdown failed: {logs}"
    );
    let identity = std::fs::read_to_string(workspace.join("identity")).unwrap();
    assert_eq!(
        identity,
        format!(
            "Git Probe\n{0}@relay.invalid\nGit Probe\n{0}@relay.invalid\n",
            keys.public_key().to_hex()
        )
    );
    let credential = std::fs::read_to_string(workspace.join("credential-result")).unwrap();
    assert!(credential.contains("authtype=Nostr"));
    let encoded = credential
        .lines()
        .find_map(|line| line.strip_prefix("credential="))
        .unwrap();
    let event: nostr::Event = serde_json::from_slice(
        &base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap(),
    )
    .unwrap();
    event.verify().unwrap();
    assert_eq!(event.pubkey, keys.public_key());
    assert!(event
        .tags
        .iter()
        .any(|tag| tag.as_slice() == ["u", "https://relay.invalid/git/test/repo"]));
    // BUZZ_PRIVATE_KEY must carry the --private-key secret (hex) verbatim.
    let buzz_private_key = std::fs::read_to_string(workspace.join("buzz_private_key")).unwrap();
    assert_eq!(
        buzz_private_key.trim(),
        keys.secret_key().to_secret_hex(),
        "BUZZ_PRIVATE_KEY must equal the --private-key argument"
    );
    // BUZZ_RELAY_URL must equal the --relay-url argument verbatim.
    let buzz_relay_url = std::fs::read_to_string(workspace.join("buzz_relay_url")).unwrap();
    assert_eq!(
        buzz_relay_url.trim(),
        "wss://relay.invalid",
        "BUZZ_RELAY_URL must equal the --relay-url argument"
    );
    for file in ["keyfile", "signer", "credential"] {
        let path = std::fs::read_to_string(workspace.join(file)).unwrap();
        assert!(
            !std::path::Path::new(path.trim()).exists(),
            "{file} survived harness shutdown"
        );
    }
}

#[test]
fn keyfile_removed_after_adapter_failure() {
    check_keyfile_removed_after_adapter_failure(false);
}

#[test]
fn task_keyfile_removed_after_adapter_failure() {
    check_keyfile_removed_after_adapter_failure(true);
}

fn check_keyfile_removed_after_adapter_failure(task: bool) {
    let temp = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let input = tempfile::NamedTempFile::new().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_buzz-acp"));
    if task {
        std::fs::write(
            input.path(),
            serde_json::json!({
                "version": 1, "taskId": "failure", "agentPubkey": keys.public_key().to_hex(),
                "prompt": "probe", "maxDurationMs": 10000
            })
            .to_string(),
        )
        .unwrap();
        command.arg("run").arg("--task").arg(input.path());
    }
    let output = command
        .args([
            "--private-key",
            &keys.secret_key().to_secret_hex(),
            "--relay-url",
            "ws://localhost:1",
            "--agent-command",
            "/missing-buzz-adapter",
        ])
        .env("TMPDIR", temp.path())
        .env_remove("BUZZ_ACP_SETUP_PAYLOAD")
        .env_remove("BUZZ_AUTH_TAG")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}
