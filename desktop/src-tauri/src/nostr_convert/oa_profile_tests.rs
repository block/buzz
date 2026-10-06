//! NIP-OA profile regressions. All keys, timestamps and Schnorr nonces are
//! synthetic and fixed; these fixtures require no clock, RNG, relay or config.

use nostr::hashes::{sha256, Hash};
use nostr::secp256k1::{schnorr::Signature, Keypair, Message};
use nostr::{Event, EventBuilder, Keys, Kind, SecretKey, Tag, Timestamp, SECP256K1};

use super::{
    profile_has_valid_oa_owner, profile_info_from_event, profile_valid_oa_owner_pubkey, tags_named,
    user_search_result_from_event, users_batch_from_events, verified_agent_owners_from_profiles,
};

const CREATED_AT: u64 = 1_700_000_000;

// Public test scalars, matching the owner/agent identities in NIP-OA's vectors.
fn keys(scalar: u8) -> Keys {
    let mut bytes = [0; 32];
    bytes[31] = scalar;
    Keys::new(SecretKey::from_slice(&bytes).unwrap())
}

fn sign(keys: &Keys, message: Message) -> Signature {
    let keypair = Keypair::from_secret_key(SECP256K1, keys.secret_key());
    SECP256K1.sign_schnorr_no_aux_rand(&message, &keypair)
}

// Intentionally bypass the SDK's *creation* validation so malformed conditions
// and self-attestation can have genuine signatures and exercise verification.
fn auth_tag_for(owner: &Keys, agent: &Keys, conditions: &str) -> Tag {
    let preimage = format!(
        "nostr:agent-auth:{}:{conditions}",
        agent.public_key().to_hex()
    );
    let digest = sha256::Hash::hash(preimage.as_bytes()).to_byte_array();
    let signature = sign(owner, Message::from_digest(digest));
    Tag::parse(vec![
        "auth".to_string(),
        owner.public_key().to_hex(),
        conditions.to_string(),
        signature.to_string(),
    ])
    .unwrap()
}

fn auth_tag(conditions: &str) -> Tag {
    auth_tag_for(&keys(1), &keys(2), conditions)
}

fn event(kind: Kind, created_at: u64, tags: Vec<Tag>) -> Event {
    let agent = keys(2);
    let mut unsigned = EventBuilder::new(kind, r#"{"display_name":"Synthetic agent"}"#)
        .tags(tags)
        .custom_created_at(Timestamp::from(created_at))
        .build(agent.public_key());
    let signature = sign(&agent, Message::from_digest(unsigned.id().to_bytes()));
    unsigned.add_signature(signature).unwrap()
}

fn profile(tags: Vec<Tag>) -> Event {
    event(Kind::Metadata, CREATED_AT, tags)
}

fn assert_ownership(event: &Event, expected: Option<String>) {
    assert_eq!(profile_valid_oa_owner_pubkey(event), expected);
    assert_eq!(profile_has_valid_oa_owner(event), expected.is_some());

    let info = profile_info_from_event(event).unwrap();
    assert_eq!(info.owner_pubkey, expected);
    assert_eq!(info.pubkey, event.pubkey.to_hex());
    let search = user_search_result_from_event(event);
    assert_eq!(search.owner_pubkey, expected);
    assert_eq!(search.is_agent, expected.is_some());
    assert_eq!(search.pubkey, event.pubkey.to_hex());
    let pubkey = event.pubkey.to_hex();
    let batch = users_batch_from_events(std::slice::from_ref(event), std::slice::from_ref(&pubkey));
    assert_eq!(batch.profiles[&pubkey].owner_pubkey, expected);
    assert_eq!(batch.profiles[&pubkey].is_agent, expected.is_some());
    let owners = verified_agent_owners_from_profiles(std::slice::from_ref(event));
    assert_eq!(owners.get(&pubkey), expected.as_ref());
}

#[test]
fn accepts_unconditional_and_applicable_conditional_ownership() {
    for conditions in [
        "",
        "kind=0",
        "created_at>1699999999&kind=0&created_at<1700000001",
        "created_at<1700000001&created_at>1699999999&kind=0&kind=0",
    ] {
        let tag = auth_tag(conditions);
        // Check that deterministic fixture signing agrees with the SDK verifier.
        let json = serde_json::to_string(tag.as_slice()).unwrap();
        assert_eq!(
            buzz_sdk_pkg::nip_oa::verify_auth_tag(&json, &keys(2).public_key()).unwrap(),
            keys(1).public_key()
        );
        assert_ownership(&profile(vec![tag]), Some(keys(1).public_key().to_hex()));
    }
}

#[test]
fn rejects_duplicate_auth_tags_including_malformed_tags_in_either_order() {
    let valid = auth_tag("");
    let malformed = Tag::parse(["auth"]).unwrap();
    for tags in [
        vec![valid.clone(), valid.clone()],
        vec![valid.clone(), auth_tag("kind=0")],
        vec![valid.clone(), malformed.clone()],
        vec![malformed, valid],
    ] {
        let event = profile(tags);
        assert_eq!(tags_named(&event, "auth").count(), 2);
        assert_ownership(&event, None);
    }
}

#[test]
fn rejects_wrong_kind_condition_and_conflicting_clauses() {
    for conditions in ["kind=1", "kind=0&kind=1", "kind=1&kind=0"] {
        assert_ownership(&profile(vec![auth_tag(conditions)]), None);
    }
}

#[test]
fn time_bounds_are_strict_and_use_event_time_not_wall_clock() {
    let tag = auth_tag("created_at>1699999999&created_at<1700000001");
    for (timestamp, accepted) in [
        (1_699_999_998, false),
        (1_699_999_999, false),
        (CREATED_AT, true),
        (1_700_000_001, false),
        (1_700_000_002, false),
        (u64::from(u32::MAX) + 1, false),
    ] {
        let event = event(Kind::Metadata, timestamp, vec![tag.clone()]);
        let expected = accepted.then(|| keys(1).public_key().to_hex());
        assert_ownership(&event, expected);
    }
}

#[test]
fn rejects_malformed_tag_shapes_and_hex() {
    let valid = auth_tag("").as_slice().to_vec();
    let mut extra = valid.clone();
    extra.push("extra".to_string());
    let mut bad_owner = valid.clone();
    bad_owner[1] = "not-a-pubkey".to_string();
    let mut uppercase_owner = valid.clone();
    uppercase_owner[1] = uppercase_owner[1].to_uppercase();
    let mut uppercase_signature = valid.clone();
    uppercase_signature[3] = uppercase_signature[3].to_uppercase();
    let mut bad_signature = valid.clone();
    bad_signature[3] = "00".repeat(64);
    for values in [
        vec!["auth".to_string()],
        valid[..3].to_vec(),
        extra,
        bad_owner,
        uppercase_owner,
        uppercase_signature,
        bad_signature,
    ] {
        assert_ownership(&profile(vec![Tag::parse(values).unwrap()]), None);
    }
}

#[test]
fn rejects_signed_but_malformed_conditions() {
    for conditions in [
        "kind=0&",
        "&kind=0",
        "kind=0&&kind=0",
        "kind=00",
        "kind=65536",
        "kind=0 ",
        "kind=٠",
        "Kind=0",
        "created_at=1700000000",
        "created_at<4294967296",
        "created_at>-1",
        "unsupported=0",
    ] {
        assert_ownership(&profile(vec![auth_tag(conditions)]), None);
    }
}

#[test]
fn rejects_absent_authority_self_attestation_and_wrong_agent_binding() {
    assert_ownership(&profile(vec![]), None);
    assert_ownership(
        &profile(vec![
            Tag::parse(["owner", &keys(1).public_key().to_hex()]).unwrap()
        ]),
        None,
    );
    assert_ownership(&profile(vec![auth_tag_for(&keys(2), &keys(2), "")]), None);
    assert_ownership(&profile(vec![auth_tag_for(&keys(1), &keys(3), "")]), None);
}

#[test]
fn rejects_non_profile_and_invalid_event_even_with_valid_auth_tag() {
    assert_ownership(&event(Kind::TextNote, CREATED_AT, vec![auth_tag("")]), None);

    let mut wrong_id = profile(vec![auth_tag("")]);
    wrong_id.content = r#"{"display_name":"Tampered"}"#.to_string();
    assert!(wrong_id.verify().is_err());
    assert_ownership(&wrong_id, None);

    let mut wrong_signature = profile(vec![auth_tag("")]);
    wrong_signature.sig = sign(
        &keys(3),
        Message::from_digest(wrong_signature.id.to_bytes()),
    );
    assert!(wrong_signature.verify().is_err());
    assert_ownership(&wrong_signature, None);
}
