#![cfg(unix)]

use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use nostr::{
    hashes::{sha256, Hash},
    secp256k1::{Keypair, Message},
    SECP256K1,
};
use serde_json::Value;

const SECRET: &str = "0000000000000000000000000000000000000000000000000000000000000003";
const SIGNER: &str = "f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9";
const OWNER: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
const OA_SIG: &str = "54b97dfd2b7d61c1bc1b5facab9d12a991fe0ac3dcb9044b3176f63bebb6f67340eb0ad866f2d5568b78b58ba234ee9f490f8c41e64a949c200315801520ed25";
const PAYLOAD: &[u8] = b"tree 4b825dc642cb6eb9a060e54bf899d69f7cb46101\nauthor Test <test@example.com> 1700000000 +0000\ncommitter Test <test@example.com> 1700000000 +0000\n\nTest\n";

struct Harness(PathBuf);

impl Harness {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "buzz-git-encoding-{}",
            nostr::Keys::generate().public_key().to_hex()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn command(&self, program: &str) -> Command {
        let mut cmd = Command::new(program);
        cmd.current_dir(&self.0)
            .env_remove("NOSTR_PRIVATE_KEY")
            .env_remove("BUZZ_PRIVATE_KEY")
            .env_remove("BUZZ_AUTH_TAG")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_COUNT", "0");
        cmd
    }

    fn verify(&self, json: &str, payload: &[u8]) -> Output {
        let sigfile = self.0.join("sig");
        fs::write(
            &sigfile,
            format!(
                "-----BEGIN SIGNED MESSAGE-----\n{}\n-----END SIGNED MESSAGE-----\n",
                STANDARD.encode(json)
            ),
        )
        .unwrap();
        pipe(
            self.command(env!("CARGO_BIN_EXE_git-sign-nostr"))
                .args(["--status-fd=1", "--verify"])
                .arg(sigfile)
                .arg("-"),
            payload,
        )
    }

    fn sign(&self, payload: &[u8]) -> String {
        let output = pipe(
            self.command(env!("CARGO_BIN_EXE_git-sign-nostr"))
                .args(["--status-fd=2", "-bsau", SIGNER])
                .env("NOSTR_PRIVATE_KEY", SECRET)
                .env(
                    "BUZZ_AUTH_TAG",
                    serde_json::json!(["auth", OWNER, "", OA_SIG]).to_string(),
                ),
            payload,
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let armor = String::from_utf8(output.stdout).unwrap();
        String::from_utf8(STANDARD.decode(armor.lines().nth(1).unwrap()).unwrap()).unwrap()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn pipe(cmd: &mut Command, payload: &[u8]) -> Output {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(payload).unwrap();
    child.wait_with_output().unwrap()
}

fn envelope(version: u64, sig: &str, t: u64, oa: bool) -> String {
    let binding = if oa {
        format!(r#","oa":["{OWNER}","","{OA_SIG}"]"#)
    } else {
        String::new()
    };
    format!(r#"{{"v":{version},"pk":"{SIGNER}","sig":"{sig}","t":{t}{binding}}}"#)
}

fn legacy_signature(payload: &[u8], oa: bool) -> String {
    let binding = if oa {
        format!("{OWNER}::{OA_SIG}:")
    } else {
        String::new()
    };
    let preimage = [
        format!("nostr:git:v1:1700000000:{binding}").as_bytes(),
        payload,
    ]
    .concat();
    let digest = sha256::Hash::hash(&preimage).to_byte_array();
    let key = Keypair::from_seckey_str(SECP256K1, SECRET).unwrap();
    let sig = SECP256K1.sign_schnorr(&Message::from_digest(digest), &key);
    envelope(1, &sig.to_string(), 1700000000, oa)
}

#[test]
fn attestation_cannot_move_into_payload_or_downgrade_version() {
    let h = Harness::new();
    let json = h.sign(PAYLOAD);
    let parsed: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["v"], 2);
    assert!(h.verify(&json, PAYLOAD).status.success());
    let sig = parsed["sig"].as_str().unwrap();
    let t = parsed["t"].as_u64().unwrap();
    let moved = [format!("{OWNER}::{OA_SIG}:").as_bytes(), PAYLOAD].concat();
    for (altered, payload) in [
        (envelope(2, sig, t, false), moved.as_slice()),
        (envelope(2, sig, t, false), PAYLOAD),
        (envelope(1, sig, t, true), PAYLOAD),
        (envelope(3, sig, t, true), PAYLOAD),
    ] {
        let output = h.verify(&altered, payload);
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("GOODSIG"));
    }
}

#[test]
fn legacy_commits_and_tags_verify_but_prefixed_payloads_do_not() {
    let h = Harness::new();
    for header in ["tree", "object"] {
        for width in [40, 64] {
            let payload = format!("{header} {}\n\nlegacy\n", "a".repeat(width));
            for oa in [false, true] {
                assert!(h
                    .verify(
                        &legacy_signature(payload.as_bytes(), oa),
                        payload.as_bytes()
                    )
                    .status
                    .success());
            }
        }
    }
    let json = legacy_signature(PAYLOAD, true);
    assert!(h.verify(&json, PAYLOAD).status.success());
    let parsed: Value = serde_json::from_str(&json).unwrap();
    let stripped = envelope(1, parsed["sig"].as_str().unwrap(), 1700000000, false);
    let moved = [format!("{OWNER}::{OA_SIG}:").as_bytes(), PAYLOAD].concat();
    let output = h.verify(&stripped, &moved);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("legacy signatures require"));
}

#[test]
fn git_creates_and_verifies_a_commit_and_tag() {
    let h = Harness::new();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "Test"],
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.signingkey", SIGNER],
        vec!["config", "gpg.format", "x509"],
        vec![
            "config",
            "gpg.x509.program",
            env!("CARGO_BIN_EXE_git-sign-nostr"),
        ],
        vec![
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--allow-empty",
            "-S",
            "-m",
            "Test",
        ],
        vec!["verify-commit", "HEAD"],
        vec!["tag", "-s", "v-test", "-m", "Test tag"],
        vec!["verify-tag", "v-test"],
    ] {
        let output = h
            .command("git")
            .args(&args)
            .env("NOSTR_PRIVATE_KEY", SECRET)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
