//! Unit tests for `build_nostr_identity_binding_event`. Split from
//! `identity.rs` for file-size discipline; wired via `#[path]`.

use super::build_nostr_identity_binding_event;
use crate::nostr_bind;
use nostr::{JsonUtil, Keys};

fn tag_values(event: &nostr::Event) -> Vec<Vec<String>> {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect()
}

#[test]
fn build_nostr_identity_binding_event_signs_exact_shape() {
    let keys = Keys::generate();
    let event = build_nostr_identity_binding_event(
        &keys,
        "550e8400-e29b-41d4-a716-446655440000",
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567",
        "123456",
        "https://example.com",
        "2999-01-01T00:00:00Z",
    )
    .unwrap();

    assert_eq!(event.kind.as_u16(), nostr_bind::KIND);
    assert_eq!(event.content, nostr_bind::CONTENT);
    assert_eq!(event.pubkey, keys.public_key());
    assert!(event.verify_id());
    assert!(event.verify_signature());
    assert!(nostr::Event::from_json(event.as_json()).is_ok());

    let tags = tag_values(&event);
    assert!(tags.contains(&vec![
        "challenge_id".into(),
        "550e8400-e29b-41d4-a716-446655440000".into(),
    ]));
    assert!(tags.contains(&vec![
        "nonce".into(),
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567".into(),
    ]));
    assert!(tags.contains(&vec!["verification_code".into(), "123456".into(),]));
    assert!(tags.contains(&vec!["audience".into(), "buzz:nostr-identity".into()]));
    assert!(tags.contains(&vec!["action".into(), "bind_nostr_identity".into(),]));
    assert!(tags.contains(&vec!["protocol".into(), "buzz-nostr-identity".into(),]));
    assert!(tags.contains(&vec!["version".into(), "1".into(),]));
    assert!(tags.contains(&vec!["origin".into(), "https://example.com".into(),]));
    assert!(tags.contains(&vec!["expires_at".into(), "2999-01-01T00:00:00Z".into(),]));
}

#[test]
fn build_nostr_identity_binding_event_rejects_malformed_verification_code() {
    let keys = Keys::generate();
    let error = build_nostr_identity_binding_event(
        &keys,
        "550e8400-e29b-41d4-a716-446655440000",
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567",
        "12345a",
        "https://example.com",
        "2999-01-01T00:00:00Z",
    )
    .unwrap_err();

    assert_eq!(error, "verification_code must be exactly 6 digits");
}

#[test]
fn build_nostr_identity_binding_event_rejects_expired_link() {
    let keys = Keys::generate();
    let error = build_nostr_identity_binding_event(
        &keys,
        "550e8400-e29b-41d4-a716-446655440000",
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567",
        "123456",
        "https://example.com",
        "2000-01-01T00:00:00Z",
    )
    .unwrap_err();

    assert_eq!(error, "expires_at is expired");
}
