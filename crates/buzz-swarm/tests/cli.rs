#![cfg(unix)]

use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

use nostr::{Keys, ToBech32};
use tempfile::TempDir;

/// Exercise the shipped CLI and real child processes without contacting a
/// community or model provider. The harness records only test identities.
struct Fixture {
    root: TempDir,
    owner: Keys,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        let script = root.path().join("harness");
        fs::write(&script, r#"#!/bin/sh
set -eu
test -z "${OWNER_SOURCE+x}"
test -z "${OTHER_AGENT_SOURCE+x}"
test -z "${SIBLING_SOURCE+x}"
test -z "${TAG_SOURCE+x}"
test -z "${BUZZ_ACP_HEARTBEAT_PROMPT+x}"
test "$BUZZ_ACP_SYSTEM_PROMPT_FILE" = "persona.txt"
test -z "$BUZZ_ACP_MCP_COMMAND"
printf '%s\n%s\n%s\n%s\n' "$BUZZ_PRIVATE_KEY" "$BUZZ_AUTH_TAG" "$BUZZ_RELAY_URL" "$(pwd -P)" > "launch-${BUZZ_RELAY_URL##*/}"
"#).expect("harness");
        fs::set_permissions(script, fs::Permissions::from_mode(0o755)).expect("chmod");
        fs::write(root.path().join("persona.txt"), "Test agent").expect("persona");
        Self {
            root,
            owner: Keys::generate(),
        }
    }

    fn write_config(&self, agents: &str) {
        fs::write(
            self.root.path().join("swarm.yaml"),
            format!(
                r#"
owner:
  nsec: {{env: OWNER_SOURCE}}
defaults:
  relays: [https://one.example, wss://two.example]
  harness: ./harness
  env:
    BUZZ_ACP_AGENT_COMMAND: /usr/bin/true
    BUZZ_ACP_MCP_COMMAND: ""
    BUZZ_ACP_SYSTEM_PROMPT_FILE: persona.txt
agents:
{agents}
  - name: sibling
    enabled: false
    auth_tag: {{env: TAG_SOURCE}}
    env:
      SIBLING_CREDENTIAL: {{env: SIBLING_SOURCE}}
"#
            ),
        )
        .expect("config");
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_buzz-swarm"));
        command
            .current_dir(std::env::temp_dir())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env(
                "OWNER_SOURCE",
                self.owner.secret_key().to_bech32().expect("nsec"),
            )
            .env("OTHER_AGENT_SOURCE", "must-not-reach-the-child")
            .env("SIBLING_SOURCE", "declared-by-a-sibling-only")
            .env("TAG_SOURCE", "declared-by-a-disabled-sibling")
            .env(
                "BUZZ_ACP_HEARTBEAT_PROMPT",
                std::ffi::OsString::from_vec(vec![0xff]),
            )
            // A non-UTF-8 ambient variable must not panic environment capture.
            .env("NON_UTF8", std::ffi::OsString::from_vec(vec![0xff]))
            .arg("--config")
            .arg(self.root.path().join("swarm.yaml"));
        command
    }

    /// Launch records land in the default per-agent workdir.
    fn launch(&self, host: &str) -> std::path::PathBuf {
        self.root
            .path()
            .join("workspaces/helper")
            .join(format!("launch-{host}"))
    }

    fn run(&self, args: &[&str]) -> Output {
        let output = self.command().args(args).output().expect("run swarm");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}

#[test]
fn starts_from_another_directory_and_reuses_one_identity_across_relays_and_restarts() {
    let fixture = Fixture::new();
    fixture.write_config("  - name: helper\n  - name: disabled\n    enabled: false\n    nsec: {env: OTHER_AGENT_SOURCE}");
    let preview = fixture.run(&["validate"]);
    assert!(String::from_utf8_lossy(&preview.stdout).contains("created on start"));
    for written in ["keys", "workspaces"] {
        assert!(
            !fixture.root.path().join(written).exists(),
            "validation wrote {written}"
        );
    }
    assert!(
        !fixture.launch("one.example").exists(),
        "validation launched a harness"
    );

    fixture.run(&[]);
    let key_file = fixture.root.path().join("keys/helper.nsec");
    let persisted = fs::read_to_string(&key_file).expect("persisted key");
    assert_eq!(
        fs::metadata(&key_file)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let keys = Keys::parse(persisted.trim()).expect("agent key");
    let first = fs::read_to_string(fixture.launch("one.example")).expect("first launch");
    let second = fs::read_to_string(fixture.launch("two.example")).expect("second launch");
    let first: Vec<_> = first.lines().collect();
    let second: Vec<_> = second.lines().collect();
    assert!(
        first[0] == persisted.trim() && first[0] == second[0],
        "identities differ"
    );
    assert_eq!(first[1], second[1], "relays received different auth tags");
    assert_ne!(first[2], second[2]);
    // Agents run in a private workdir, not beside `keys/`.
    let workdir = fixture.root.path().join("workspaces/helper");
    assert_eq!(
        Path::new(first[3]),
        workdir.canonicalize().expect("workdir")
    );
    assert_eq!(
        fs::metadata(&workdir)
            .expect("workdir")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        buzz_sdk::nip_oa::verify_auth_tag(first[1], &keys.public_key()).expect("tag"),
        fixture.owner.public_key()
    );

    fixture.run(&[]);
    assert!(
        fs::read_to_string(key_file).expect("key after restart") == persisted,
        "identity changed on restart"
    );
}

#[test]
fn accepts_a_presigned_tag_without_the_owner_key() {
    let fixture = Fixture::new();
    let agent = Keys::generate();
    let tag =
        buzz_sdk::nip_oa::compute_auth_tag(&fixture.owner, &agent.public_key(), "").expect("tag");
    fixture.write_config(
        "  - name: helper\n    nsec: {file: supplied.nsec}\n    auth_tag: {file: auth.json}",
    );
    fs::write(
        fixture.root.path().join("supplied.nsec"),
        agent.secret_key().to_bech32().expect("nsec"),
    )
    .expect("key");
    fs::write(fixture.root.path().join("auth.json"), tag).expect("tag file");
    let path = fixture.root.path().join("swarm.yaml");
    let config = fs::read_to_string(&path)
        .expect("config")
        .replace("owner:\n  nsec: {env: OWNER_SOURCE}\n", "");
    fs::write(path, config).expect("without owner");
    let output = fixture
        .command()
        .env_remove("OWNER_SOURCE")
        .env_remove("OTHER_AGENT_SOURCE")
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!fixture.root.path().join("keys").exists());
}

#[test]
fn invalid_launches_start_nothing_and_persist_no_keys() {
    for agents in [
        "  - name: helper\n    relays: [https://one.example, wss://one.example/]",
        "  - name: helper\n  - name: other\n    harness: /missing/harness",
        "  - name: ../outside",
        "  - name: helper\n    relays: []",
    ] {
        let fixture = Fixture::new();
        fixture.write_config(agents);
        let output = fixture.command().output().expect("run");
        assert!(!output.status.success(), "invalid configuration succeeded");
        assert!(!fixture.root.path().join("keys").exists());
        assert!(!fixture.root.path().join("workspaces").exists());
    }
}

#[test]
fn unknown_selection_and_mismatched_delegation_fail_closed() {
    let fixture = Fixture::new();
    fixture.write_config("  - name: helper");
    let output = fixture
        .command()
        .args(["--only", "helper,typo", "validate"])
        .output()
        .expect("run");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("typo"));

    let agent = Keys::generate();
    let stranger = Keys::generate();
    let tag = buzz_sdk::nip_oa::compute_auth_tag(&fixture.owner, &stranger.public_key(), "")
        .expect("tag");
    fixture.write_config(
        "  - name: helper\n    nsec: {file: supplied.nsec}\n    auth_tag: {file: auth.json}",
    );
    fs::write(
        fixture.root.path().join("supplied.nsec"),
        agent.secret_key().to_bech32().expect("nsec"),
    )
    .expect("key");
    fs::write(fixture.root.path().join("auth.json"), tag).expect("tag file");
    assert!(!fixture.command().output().expect("run").status.success());
    assert!(!fixture.launch("one.example").exists());
}

#[test]
fn an_expired_delegation_fails_before_launch() {
    let fixture = Fixture::new();
    fixture.write_config("  - name: helper");
    let path = fixture.root.path().join("swarm.yaml");
    let config = fs::read_to_string(&path)
        .expect("read")
        .replace("owner:\n", "owner:\n  conditions: created_at<1\n");
    fs::write(path, config).expect("write");
    assert!(!fixture.command().output().expect("run").status.success());
    assert!(!fixture.root.path().join("keys").exists());
    assert!(!fixture.launch("one.example").exists());
}
