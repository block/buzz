use super::*;
use serde_json::json;

fn pending() -> Content {
    Content {
        status: Status::Pending,
        note: Some("Inspect experiment X; decide continue/revise/stop".into()),
        target: None,
        extra: Default::default(),
    }
}

fn event(keys: &Keys, id: &str, content: &Content, due: Option<u64>, created: u64) -> Event {
    build(keys, id, content, due, created)
        .unwrap()
        .sign_with_keys(keys)
        .unwrap()
}

#[test]
fn encrypted_lifecycle_roundtrip_and_time_boundary() {
    let keys = Keys::generate();
    let initial = event(&keys, "opaque-id", &pending(), Some(200), 100);
    assert!(!initial.content.contains("Inspect"));
    assert!(initial
        .tags
        .iter()
        .all(|tag| !matches!(tag.kind().as_str(), "h" | "e" | "p")));
    let reminder = Reminder::decrypt(&initial, &keys).unwrap();
    assert!(!reminder.is_due(199));
    assert!(reminder.is_due(200));
    assert!(Reminder::decrypt(&initial, &Keys::generate()).is_err());

    let snoozed = event(&keys, "opaque-id", &pending(), Some(400), 201);
    let heads = current_heads(vec![initial.clone(), snoozed.clone(), initial], &keys);
    assert_eq!(heads.len(), 1);
    assert!(!heads[0].is_due(200));
    let mut completed = pending();
    completed.status = Status::Done;
    let done = event(&keys, "opaque-id", &completed, None, 401);
    assert!(done.tags.expiration().is_some());
    let heads = current_heads(vec![done, snoozed], &keys);
    assert_eq!(heads[0].content.status, Status::Done);
    assert!(!heads[0].is_due(u64::MAX));
}

#[test]
fn malformed_new_head_never_resurrects_previous_intent() {
    let keys = Keys::generate();
    let previous = event(&keys, "id", &pending(), Some(2), 1);
    let bad = EventBuilder::new(Kind::Custom(30300), "not ciphertext")
        .tags([Tag::identifier("id")])
        .custom_created_at(Timestamp::from(3))
        .sign_with_keys(&keys)
        .unwrap();
    assert!(current_heads(vec![bad, previous], &keys).is_empty());
}

#[test]
fn same_second_replacement_converges_on_lowest_id() {
    let keys = Keys::generate();
    let first = event(&keys, "id", &pending(), Some(20), 10);
    let second = event(&keys, "id", &pending(), Some(30), 10);
    let winner = first.id.min(second.id).to_hex();
    assert_eq!(
        current_heads(vec![first.clone(), second.clone()], &keys)[0].event_id,
        winner
    );
    assert_eq!(
        current_heads(vec![second, first], &keys)[0].event_id,
        winner
    );
}

#[test]
fn bookmarks_and_terminal_events_never_wake() {
    let keys = Keys::generate();
    let bookmark = event(&keys, "bookmark", &pending(), None, 1);
    assert!(!Reminder::decrypt(&bookmark, &keys).unwrap().is_due(10));
    let mut content = pending();
    content.status = Status::Cancelled;
    assert!(build(&keys, "bad", &content, Some(3), 1).is_err());
    let ciphertext = nip44::encrypt(
        keys.secret_key(),
        &keys.public_key(),
        serde_json::to_string(&content).unwrap(),
        nip44::Version::V2,
    )
    .unwrap();
    let terminal = EventBuilder::new(Kind::Custom(30300), ciphertext)
        .tags([
            Tag::identifier("cancelled"),
            Tag::parse(["not_before", "2"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    assert!(!Reminder::decrypt(&terminal, &keys).unwrap().is_due(10));
}

#[test]
fn desktop_and_nip_targets_are_compatible_and_updates_preserve_unknown_fields() {
    let keys = Keys::generate();
    for target in [
        json!({"eventId":"ab".repeat(32),"channelId":uuid::Uuid::new_v4().to_string(),"preview":"A message","authorPubkey":keys.public_key().to_hex()}),
        json!({"id":"ab".repeat(32),"preview":"A message"}),
    ] {
        let mut content = pending();
        content.target = Some(target.clone());
        content.note = None;
        content.extra.insert("future-field".into(), json!({"a":1}));
        let decoded =
            Reminder::decrypt(&event(&keys, "id", &content, Some(20), 10), &keys).unwrap();
        assert_eq!(decoded.content.target, Some(target));
        assert_eq!(decoded.content.extra["future-field"], json!({"a":1}));
    }
}

#[test]
fn bad_content_signatures_and_duplicate_tags_are_rejected() {
    let keys = Keys::generate();
    for plaintext in [
        r#"{"status":"pending","note":"one","note":"two"}"#,
        r#"{"status":"pending","note":"valid","extra":{"a":1,"a":2}}"#,
        r#"{"status":"pending","note":"","target":{"id":"wrong"}}"#,
        r#"{"status":"unknown","note":"text"}"#,
        r#"{"status":"pending","note":42}"#,
    ] {
        let ciphertext = nip44::encrypt(
            keys.secret_key(),
            &keys.public_key(),
            plaintext,
            nip44::Version::V2,
        )
        .unwrap();
        let event = EventBuilder::new(Kind::Custom(30300), ciphertext)
            .tags([Tag::identifier("id")])
            .sign_with_keys(&keys)
            .unwrap();
        assert!(
            Reminder::decrypt(&event, &keys).is_err(),
            "accepted {plaintext}"
        );
    }
    let duplicate = build(&keys, "id", &pending(), Some(10), 1)
        .unwrap()
        .tag(Tag::parse(["not_before", "11"]).unwrap())
        .sign_with_keys(&keys)
        .unwrap();
    assert!(Reminder::decrypt(&duplicate, &keys).is_err());
    let mut tampered = event(&keys, "id", &pending(), Some(10), 1);
    tampered.content.push('x');
    assert!(Reminder::decrypt(&tampered, &keys).is_err());
}

#[test]
fn not_before_parser_rejects_lossy_or_ambiguous_values() {
    for value in [
        "",
        "01",
        " 1",
        "1 ",
        "+1",
        "-1",
        "1.0",
        "1e3",
        "9007199254740992",
        "18446744073709551616",
        "١",
    ] {
        assert!(parse_not_before(value).is_err(), "accepted {value}");
    }
    assert_eq!(parse_not_before("0").unwrap(), 0);
    assert_eq!(
        parse_not_before("9007199254740991").unwrap(),
        9_007_199_254_740_991
    );
}

#[test]
fn an_expired_pending_reminder_cannot_wake_even_if_replayed() {
    let keys = Keys::generate();
    let event = build(&keys, "expiring", &pending(), Some(20), 10)
        .unwrap()
        .tag(Tag::expiration(Timestamp::from(30)))
        .sign_with_keys(&keys)
        .unwrap();
    let reminder = Reminder::decrypt(&event, &keys).unwrap();
    assert!(reminder.is_due(29));
    assert!(!reminder.is_due(30));
}
