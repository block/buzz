//! Local-only Buzz agent identity and NIP-OA provisioning.
//!
//! The owner key is accepted on stdin, never as an argument or environment
//! variable. Only the generated agent key and signed auth tag are printed.

use std::io::{BufRead, Write};

use nostr::{Keys, ToBech32};
use zeroize::Zeroize;

pub fn run() -> i32 {
    match provision(std::io::stdin().lock(), &mut std::io::stdout()) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{}", serde_json::json!({"error": error}));
            1
        }
    }
}

fn provision<R: BufRead, W: Write>(mut input: R, output: &mut W) -> Result<(), String> {
    let mut owner_secret = String::new();
    input
        .read_line(&mut owner_secret)
        .map_err(|error| format!("failed to read owner key from stdin: {error}"))?;
    let owner = Keys::parse(owner_secret.trim())
        .map_err(|error| format!("invalid owner key from stdin: {error}"));
    owner_secret.zeroize();
    let owner = owner?;

    let agent = Keys::generate();
    let auth_tag = buzz_sdk::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "")
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
    use nostr::Keys;

    #[test]
    fn provision_emits_verifiable_agent_identity_without_owner_secret() {
        let owner = Keys::parse("0000000000000000000000000000000000000000000000000000000000000001")
            .unwrap();
        let owner_secret = owner.secret_key().to_secret_hex();
        let mut output = Vec::new();

        provision(format!("{owner_secret}\n").as_bytes(), &mut output).unwrap();

        let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
        let agent_nsec = value["agent_private_key_nsec"].as_str().unwrap();
        let agent = Keys::parse(agent_nsec).unwrap();
        let auth_tag = value["auth_tag"].as_str().unwrap();
        let verified_owner = buzz_sdk::nip_oa::verify_auth_tag(auth_tag, &agent.public_key())
            .expect("generated auth tag must verify");

        assert_eq!(value["agent_pubkey"], agent.public_key().to_hex());
        assert_eq!(value["owner_pubkey"], owner.public_key().to_hex());
        assert_eq!(verified_owner, owner.public_key());
        assert!(!String::from_utf8(output).unwrap().contains(&owner_secret));
    }

    #[test]
    fn provision_rejects_empty_owner_key_without_output() {
        let mut output = Vec::new();
        let error = provision("\n".as_bytes(), &mut output).unwrap_err();
        assert!(error.contains("invalid owner key"));
        assert!(output.is_empty());
    }
}
