use super::*;
use crate::AuditAction;

fn entry() -> AuditEntry {
    AuditEntry {
        hash_version: CURRENT_HASH_VERSION,
        community_id: uuid::Uuid::from_u128(1),
        seq: 1,
        hash: Vec::new(),
        prev_hash: None,
        action: AuditAction::EventCreated,
        actor_pubkey: Some(vec![0xab; 32]),
        object_id: Some("a".repeat(64)),
        detail: serde_json::json!({"event_kind": 9, "channel_id": null}),
        created_at: "2026-01-01T00:00:00Z".parse().unwrap(),
    }
}

#[test]
fn legacy_digest_is_preserved() {
    let mut legacy = entry();
    legacy.hash_version = 1;
    // Independently computed using the original concatenation format.
    assert_eq!(
        hex::encode(compute_hash(&legacy).unwrap()),
        "9a14ea40b48dbd85d321cf214c360ae93937d5c146a2a49586847a4317a32345"
    );
}

#[test]
fn moving_digits_between_object_and_detail_changes_hash() {
    let mut first = entry();
    first.object_id = Some("record1".into());
    first.detail = serde_json::json!(23);
    let mut second = first.clone();
    second.object_id = Some("record12".into());
    second.detail = serde_json::json!(3);
    assert_ne!(
        compute_hash(&first).unwrap(),
        compute_hash(&second).unwrap()
    );

    // Reinterpreting either row as the old encoding must not resurrect the
    // original collision: legacy identifiers have a restricted grammar.
    first.hash_version = 1;
    second.hash_version = 1;
    assert!(compute_hash(&first).is_err());
    assert!(compute_hash(&second).is_err());
}

#[test]
fn actor_cannot_absorb_an_object_presence_byte() {
    for version in [1, CURRENT_HASH_VERSION] {
        let mut original = entry();
        original.hash_version = version;
        original.actor_pubkey.as_mut().unwrap()[31] = 1;
        assert!(compute_hash(&original).is_ok());
        let mut altered = original.clone();
        altered.actor_pubkey.as_mut().unwrap().pop();
        altered.object_id = Some(format!("\u{1}{}", original.object_id.unwrap()));
        assert!(matches!(
            compute_hash(&altered),
            Err(AuditError::InvalidField { .. })
        ));
    }
}

#[test]
fn optional_values_and_version_are_bound() {
    let original = entry();
    let mut changed = original.clone();
    changed.prev_hash = Some(GENESIS_HASH.to_vec());
    assert_ne!(
        compute_hash(&original).unwrap(),
        compute_hash(&changed).unwrap()
    );
    changed.hash_version = 1;
    assert!(compute_hash(&changed).is_err());
    changed = original.clone();
    changed.hash_version = 1;
    assert_ne!(
        compute_hash(&original).unwrap(),
        compute_hash(&changed).unwrap()
    );
    changed.hash_version = 3;
    assert!(matches!(
        compute_hash(&changed),
        Err(AuditError::UnsupportedHashVersion { version: 3 })
    ));
}

#[test]
fn legacy_identifiers_have_unambiguous_boundaries() {
    let mut row = entry();
    row.hash_version = 1;
    for id in [
        Some("a".repeat(64)),
        Some(uuid::Uuid::nil().to_string()),
        None,
    ] {
        row.object_id = id;
        assert!(compute_hash(&row).is_ok());
    }
    for id in [
        "".to_string(),
        "a".repeat(63),
        "A".repeat(64),
        "record1".into(),
    ] {
        row.object_id = Some(id);
        assert!(compute_hash(&row).is_err());
    }
}

#[test]
fn stored_hash_and_actor_widths_are_checked() {
    for version in [1, CURRENT_HASH_VERSION] {
        for length in [0, 31, 33] {
            let mut row = entry();
            row.hash_version = version;
            row.actor_pubkey = Some(vec![1; length]);
            assert!(compute_hash(&row).is_err());
            row.actor_pubkey = None;
            row.seq = 2;
            row.prev_hash = Some(vec![1; length]);
            assert!(compute_hash(&row).is_err());
        }
    }
}
