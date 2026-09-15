//! Local-only Buzz agent identity and NIP-OA provisioning.
//!
//! The owner key comes from Buzz Desktop's macOS Keychain item. It is never
//! accepted through argv or the environment. Only the generated agent key and
//! signed auth tag are printed.

use std::io::{BufRead, Write};

use nostr::{Keys, ToBech32};
#[cfg(any(target_os = "macos", test))]
use serde::Deserialize;
use zeroize::Zeroize;

#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "buzz-desktop";
#[cfg(target_os = "macos")]
const KEYCHAIN_ACCOUNT: &str = "secrets";

#[cfg(any(target_os = "macos", test))]
#[derive(Deserialize)]
struct KeychainBlob {
    identity: String,
}

pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = String>,
{
    match run_inner(args, std::io::stdin().lock(), &mut std::io::stdout()) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{}", serde_json::json!({"error": error}));
            1
        }
    }
}

fn run_inner<I, R, W>(args: I, mut input: R, output: &mut W) -> Result<(), String>
where
    I: IntoIterator<Item = String>,
    R: BufRead,
    W: Write,
{
    let mut agent_from_stdin = false;
    for arg in args {
        match arg.as_str() {
            "--agent-private-key-stdin" if !agent_from_stdin => agent_from_stdin = true,
            _ => return Err(format!("unknown or duplicate argument: {arg}")),
        }
    }

    let owner = load_owner_keys()?;
    let agent = if agent_from_stdin {
        let mut value = String::new();
        input
            .read_line(&mut value)
            .map_err(|error| format!("failed to read agent key from stdin: {error}"))?;
        let agent = Keys::parse(value.trim())
            .map_err(|error| format!("invalid agent key from stdin: {error}"));
        value.zeroize();
        agent?
    } else {
        Keys::generate()
    };

    write_provisioned_identity(&owner, &agent, output)
}

#[cfg(target_os = "macos")]
fn load_owner_keys() -> Result<Keys, String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
        .map_err(|error| format!("failed to open Buzz Desktop Keychain entry: {error}"))?;
    let mut raw = entry
        .get_password()
        .map_err(|error| format!("failed to read Buzz Desktop Keychain entry: {error}"))?;
    let blob: Result<KeychainBlob, _> = serde_json::from_str(&raw);
    raw.zeroize();
    let mut blob = blob.map_err(|error| format!("invalid Buzz Desktop Keychain blob: {error}"))?;
    let owner = Keys::parse(blob.identity.trim())
        .map_err(|error| format!("invalid owner key in Buzz Desktop Keychain: {error}"));
    blob.identity.zeroize();
    owner
}

#[cfg(not(target_os = "macos"))]
fn load_owner_keys() -> Result<Keys, String> {
    Err("Buzz Desktop Keychain provisioning is supported only on macOS".to_string())
}

fn write_provisioned_identity<W: Write>(
    owner: &Keys,
    agent: &Keys,
    output: &mut W,
) -> Result<(), String> {
    let auth_tag = buzz_sdk::nip_oa::compute_auth_tag(owner, &agent.public_key(), "")
        .map_err(|error| format!("failed to compute owner auth tag: {error}"))?;
    let agent_nsec = agent
        .secret_key()
        .to_bech32()
        .map_err(|error| format!("failed to encode agent private key: {error}"))?;

    serde_json::to_writer(
        &mut *output,
        &serde_json::json!({
            "agent_private_key_nsec": agent_nsec,
            "agent_pubkey": agent.public_key().to_hex(),
            "owner_pubkey": owner.public_key().to_hex(),
            "auth_tag": auth_tag,
        }),
    )
    .map_err(|error| format!("failed to serialize provisioned identity: {error}"))?;
    output
        .write_all(b"\n")
        .map_err(|error| format!("failed to write provisioned identity: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner() -> Keys {
        Keys::parse("0000000000000000000000000000000000000000000000000000000000000001").unwrap()
    }

    #[test]
    fn provisioned_identity_has_verifiable_owner_tag() {
        let owner = owner();
        let agent = Keys::generate();
        let owner_secret = owner.secret_key().to_secret_hex();
        let mut output = Vec::new();

        write_provisioned_identity(&owner, &agent, &mut output).unwrap();

        let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
        let agent_nsec = value["agent_private_key_nsec"].as_str().unwrap();
        let parsed_agent = Keys::parse(agent_nsec).unwrap();
        let auth_tag = value["auth_tag"].as_str().unwrap();
        let verified_owner =
            buzz_sdk::nip_oa::verify_auth_tag(auth_tag, &parsed_agent.public_key())
                .expect("generated auth tag must verify");

        assert_eq!(parsed_agent.public_key(), agent.public_key());
        assert_eq!(value["owner_pubkey"], owner.public_key().to_hex());
        assert_eq!(verified_owner, owner.public_key());
        assert!(!String::from_utf8(output).unwrap().contains(&owner_secret));
    }

    #[test]
    fn existing_agent_stdin_parser_rejects_unknown_arguments_before_keychain_read() {
        let mut output = Vec::new();
        let error = run_inner(["--wrong".to_string()], "".as_bytes(), &mut output).unwrap_err();
        assert_eq!(error, "unknown or duplicate argument: --wrong");
        assert!(output.is_empty());
    }

    #[test]
    fn keychain_blob_ignores_other_secret_entries() {
        let raw = r#"{"identity":"owner","agent:abc":"must-not-be-retained"}"#;
        let blob: KeychainBlob = serde_json::from_str(raw).unwrap();
        assert_eq!(blob.identity, "owner");
    }
}
