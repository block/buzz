//! Compute a NIP-OA auth tag for an agent keypair.
//!
//! Usage:
//!   cargo run --release --example compute_auth_tag -- <owner_nsec> <agent_npub> [conditions]
//!
//! Prints the JSON auth tag to stdout.

use buzz_core::nostr_identity::{
    parse_public_key_compat, parse_secret_key_compat, KeyInputEncoding,
};
use buzz_sdk::nip_oa;
use nostr::Keys;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <owner_nsec> <agent_npub> [conditions]", args[0]);
        std::process::exit(1);
    }

    let (owner_secret, owner_encoding) = match parse_secret_key_compat(&args[1]) {
        Ok(value) => value,
        Err(_) => {
            eprintln!("invalid owner nsec");
            std::process::exit(1);
        }
    };
    let (agent_pubkey, agent_encoding) = match parse_public_key_compat(&args[2]) {
        Ok(value) => value,
        Err(_) => {
            eprintln!("invalid agent npub");
            std::process::exit(1);
        }
    };
    if owner_encoding == KeyInputEncoding::LegacyHex {
        eprintln!("warning: legacy owner secret hex is deprecated; use nsec");
    }
    if agent_encoding == KeyInputEncoding::LegacyHex {
        eprintln!("warning: legacy agent public-key hex is deprecated; use npub");
    }

    let owner_keys = Keys::new(owner_secret);
    let conditions = args.get(3).map(|s| s.as_str()).unwrap_or("");

    let tag_json = match nip_oa::compute_auth_tag(&owner_keys, &agent_pubkey, conditions) {
        Ok(value) => value,
        Err(_) => {
            eprintln!("failed to compute auth tag");
            std::process::exit(1);
        }
    };

    println!("{tag_json}");
}
