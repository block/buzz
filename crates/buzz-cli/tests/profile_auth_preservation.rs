//! Compile the signing seam as a sibling module so this regression can inspect
//! the final event without opening sockets or publishing anything.

#[allow(dead_code)]
#[path = "../src/client.rs"]
mod client;
#[allow(dead_code)]
#[path = "../src/error.rs"]
mod error;

use nostr::{Keys, Tag};
use serde_json::{json, Value};

#[test]
fn delegated_profile_signing_preserves_exactly_one_authorization_tag() {
    let owner_secret = "11".repeat(32);
    let agent_secret = "22".repeat(32);
    let owner_keys = Keys::parse(&owner_secret).expect("synthetic owner key must be valid");
    let agent_keys = Keys::parse(&agent_secret).expect("synthetic agent key must be valid");
    let auth_tag_json =
        buzz_sdk::nip_oa::compute_auth_tag(&owner_keys, &agent_keys.public_key(), "kind=0")
            .expect("synthetic owner attestation must be valid");
    let auth_tag_parts: Vec<String> =
        serde_json::from_str(&auth_tag_json).expect("synthetic auth tag must be JSON");
    let auth_tag = Tag::parse(auth_tag_parts).expect("synthetic auth tag must be a Nostr tag");
    let client = client::BuzzClient::new(
        "https://example.invalid".into(),
        agent_keys,
        Some(auth_tag),
        Some(auth_tag_json.clone()),
    )
    .expect("synthetic client must build");

    let profile = buzz_sdk::build_profile(
        Some("Renamed Synthetic Agent"),
        None,
        Some("https://example.invalid/avatar.png"),
        Some("preserve me"),
        Some("synthetic@example.invalid"),
    )
    .expect("synthetic profile must build");
    let signed = client
        .sign_event(profile)
        .expect("delegated profile must sign");

    signed
        .verify()
        .expect("delegated profile signature must verify");
    assert_eq!(signed.kind.as_u16(), 0);

    let expected_auth: Value =
        serde_json::from_str(&auth_tag_json).expect("synthetic auth tag must be JSON");
    let auth_tags: Vec<Value> = signed
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("auth"))
        .map(|tag| json!(tag.as_slice()))
        .collect();
    assert_eq!(auth_tags, vec![expected_auth]);

    let content: Value =
        serde_json::from_str(&signed.content).expect("profile content must be JSON");
    assert_eq!(content["display_name"], "Renamed Synthetic Agent");
    assert_eq!(content["picture"], "https://example.invalid/avatar.png");
    assert_eq!(content["about"], "preserve me");
    assert_eq!(content["nip05"], "synthetic@example.invalid");
}
