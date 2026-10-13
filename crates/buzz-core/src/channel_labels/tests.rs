use super::*;
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag};
use uuid::Uuid;

fn event(kind: u16, tags: Vec<Vec<String>>, keys: &Keys) -> Event {
    EventBuilder::new(Kind::Custom(kind), "")
        .tags(
            tags.into_iter()
                .map(|parts| Tag::parse(parts).expect("tag")),
        )
        .sign_with_keys(keys)
        .expect("sign")
}

fn tag(name: &str, value: &str) -> Vec<String> {
    vec![name.into(), value.into()]
}

fn command(kind: u16, operations: Vec<Vec<String>>) -> Event {
    let mut tags = vec![tag("h", &Uuid::new_v4().to_string())];
    tags.extend(operations);
    event(kind, tags, &Keys::generate())
}

#[test]
fn values_are_bounded_ascii_and_never_normalized() {
    for valid in [
        "a",
        "0",
        "team:infra",
        "workflow/build",
        "a._:/-9",
        "stream",
    ] {
        assert!(validate_value(valid).is_ok(), "{valid}");
    }
    assert!(validate_value(&"a".repeat(64)).is_ok());
    for invalid in [
        "", "UPPER", " a", "a ", "a\n", "a\0", "é", "a💀", "-a", ".a", ":a", "a@b",
    ] {
        assert!(validate_value(invalid).is_err(), "{invalid:?}");
    }
    assert!(validate_value(&"a".repeat(65)).is_err());
    for byte in 0..=255u8 {
        if let Ok(value) = String::from_utf8(vec![b'a', byte]) {
            assert_eq!(
                validate_value(&value).is_ok(),
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._:/-".contains(&byte)
            );
        }
    }
}

#[test]
fn sets_deduplicate_sort_and_enforce_distinct_limit() {
    assert_eq!(
        LabelSet::new(["z".into(), "a".into(), "a".into()])
            .unwrap()
            .values(),
        &["a", "z"]
    );
    assert!(LabelSet::new((0..32).map(|i| format!("label:{i}"))).is_ok());
    assert!(LabelSet::new((0..33).map(|i| format!("label:{i}"))).is_err());
    assert!(LabelSet::default().snapshot_tags().is_empty());
    assert_eq!(
        LabelSet::new(["stream".into()]).unwrap().snapshot_tags(),
        vec![vec!["L", "nip-cl"], vec!["l", "stream", "nip-cl"]]
    );
}

#[test]
fn raw_limit_is_before_deduplication() {
    for kind in [9002, 9007] {
        let name = if kind == 9007 { "label" } else { "add-label" };
        assert!(LabelCommand::parse(&command(kind, vec![tag(name, "a"); 64])).is_ok());
        assert!(LabelCommand::parse(&command(kind, vec![tag(name, "a"); 65])).is_err());
        for malformed in [
            vec![name.to_string()],
            vec![name.into(), "a".into(), "extra".into()],
        ] {
            assert!(LabelCommand::parse(&command(kind, vec![malformed])).is_err());
        }
    }
}

#[test]
fn classify_legacy_create_ordinary_metadata_and_unlabeled_h_create() {
    let keys = Keys::generate();
    assert_eq!(
        LabelCommand::parse(&event(9007, vec![tag("name", "legacy")], &keys)),
        Ok(None)
    );
    assert_eq!(
        LabelCommand::parse(&command(9002, vec![tag("topic", "hello")])),
        Ok(None)
    );
    assert!(matches!(LabelCommand::parse(&command(9007, vec![])),
        Ok(Some(LabelCommand::Create { labels, .. })) if labels.is_empty()));
    assert_eq!(
        LabelCommand::parse(&command(9, vec![tag("t", "free")])),
        Ok(None)
    );
}

#[test]
fn misplaced_tags_fail_even_without_valid_operations() {
    for kind in [9002, 9007] {
        let misplaced = if kind == 9007 {
            vec!["add-label", "remove-label", "l", "L", "t"]
        } else {
            vec!["label", "l", "L", "t"]
        };
        for name in misplaced {
            assert!(
                LabelCommand::parse(&command(kind, vec![tag(name, "a")])).is_err(),
                "{kind} {name}"
            );
        }
    }
}

#[test]
fn covered_commands_require_one_exact_non_nil_h_tag() {
    for kind in [9002, 9007] {
        let operation = tag(if kind == 9007 { "label" } else { "add-label" }, "a");
        for ids in [
            vec![],
            vec![vec!["h".into()]],
            vec![tag("h", "bad")],
            vec![tag("h", &Uuid::nil().to_string())],
            vec![tag("h", &Uuid::new_v4().to_string()); 2],
            vec![vec!["h".into(), Uuid::new_v4().to_string(), "extra".into()]],
        ] {
            let mut tags = ids;
            tags.push(operation.clone());
            assert!(LabelCommand::parse(&event(kind, tags, &Keys::generate())).is_err());
        }
    }
}

#[test]
fn label_mutations_reject_metadata_unknowns_overlap_and_content() {
    for name in [
        "name",
        "about",
        "visibility",
        "archived",
        "topic",
        "purpose",
        "ttl",
        "channel_type",
        "unknown",
        "p",
    ] {
        assert!(
            LabelCommand::parse(&command(9002, vec![tag("add-label", "a"), tag(name, "x")]))
                .is_err(),
            "{name}"
        );
    }
    assert!(LabelCommand::parse(&command(
        9002,
        vec![tag("add-label", "a"), tag("remove-label", "a")]
    ))
    .is_err());
    let mut has_content = command(9002, vec![tag("add-label", "a")]);
    has_content.content = "metadata".into();
    assert!(LabelCommand::parse(&has_content).is_err());
    for name in ["auth", "client", "client-id", "nonce", "expiration", "-"] {
        assert!(
            LabelCommand::parse(&command(9002, vec![tag("add-label", "a"), tag(name, "x")]))
                .is_ok()
        );
    }
}

#[test]
fn mutation_uses_final_size_and_preserves_unrelated_values() {
    let current = LabelSet::new((0..32).map(|i| format!("label:{i}"))).unwrap();
    let parse = |ops| LabelCommand::parse(&command(9002, ops)).unwrap().unwrap();
    let changed = current
        .apply(&parse(vec![
            tag("add-label", "new"),
            tag("remove-label", "label:0"),
        ]))
        .unwrap();
    assert_eq!(changed.values().len(), 32);
    assert!(changed.values().contains(&"label:1".into()));
    assert!(changed.values().contains(&"new".into()));
    assert!(!changed.values().contains(&"label:0".into()));
    assert!(current
        .apply(&parse(vec![tag("add-label", "new")]))
        .is_err());
    assert_eq!(
        current
            .apply(&parse(vec![
                tag("add-label", "label:0"),
                tag("remove-label", "absent")
            ]))
            .unwrap(),
        current
    );
    // More than 32 absent removals are valid: only the resulting set is bounded.
    assert_eq!(
        current
            .apply(&parse(
                (0..64)
                    .map(|i| tag("remove-label", &format!("absent:{i}")))
                    .collect()
            ))
            .unwrap(),
        current
    );
}

fn snapshot_tags(channel: Uuid) -> Vec<Vec<String>> {
    vec![tag("d", &channel.to_string()), tag("t", "stream")]
}

#[test]
fn snapshots_validate_trust_and_complete_canonical_representation() {
    let keys = Keys::generate();
    let channel = Uuid::new_v4();
    let labels = LabelSet::new(["a".into(), "stream".into()]).unwrap();
    let mut tags = snapshot_tags(channel);
    tags.extend(labels.snapshot_tags());
    let valid = event(39000, tags.clone(), &keys);
    assert_eq!(
        verify_snapshot(&valid, keys.public_key(), channel),
        Ok(labels)
    );
    assert!(verify_snapshot(&valid, Keys::generate().public_key(), channel).is_err());
    assert!(verify_snapshot(&valid, keys.public_key(), Uuid::new_v4()).is_err());
    for (field, value) in [
        ("content", serde_json::json!("tampered")),
        ("sig", serde_json::json!("0".repeat(128))),
    ] {
        let mut json = serde_json::to_value(&valid).unwrap();
        json[field] = value;
        let corrupted = Event::from_json(json.to_string()).unwrap();
        assert!(verify_snapshot(&corrupted, keys.public_key(), channel).is_err());
    }
    for kind in [9, 9002, 39001] {
        assert!(verify_snapshot(
            &event(kind, tags.clone(), &keys),
            keys.public_key(),
            channel
        )
        .is_err());
    }
}

#[test]
fn invalid_snapshot_is_never_accepted_as_empty() {
    let keys = Keys::generate();
    let channel = Uuid::new_v4();
    let l = |v: &str| vec!["l".into(), v.into(), "nip-cl".into()];
    for bad in [
        vec![tag("label", "a")],
        vec![tag("L", "nip-cl")],
        vec![l("a")],
        vec![tag("L", "other"), l("a")],
        vec![
            tag("L", "nip-cl"),
            vec!["l".into(), "a".into(), "other".into()],
        ],
        vec![tag("L", "nip-cl"), tag("l", "a")],
        vec![tag("L", "nip-cl"), l("a"), l("a")],
        vec![tag("L", "nip-cl"), l("z"), l("a")],
        vec![tag("L", "nip-cl"), l("A")],
        vec![tag("L", "nip-cl"), tag("L", "nip-cl"), l("a")],
        vec![tag("t", "forum")],
        vec![tag("d", &channel.to_string())],
    ] {
        let mut tags = snapshot_tags(channel);
        tags.extend(bad);
        assert!(verify_snapshot(&event(39000, tags, &keys), keys.public_key(), channel).is_err());
    }
    for channel_type in ["stream", "forum", "dm", "workflow"] {
        let tags = vec![tag("d", &channel.to_string()), tag("t", channel_type)];
        assert_eq!(
            verify_snapshot(
                &event(39000, tags.clone(), &keys),
                keys.public_key(),
                channel
            ),
            Ok(LabelSet::default())
        );
        let mut labeled = tags;
        labeled.extend([tag("L", "nip-cl"), l("a")]);
        assert_eq!(
            verify_snapshot(&event(39000, labeled, &keys), keys.public_key(), channel).is_ok(),
            matches!(channel_type, "stream" | "forum")
        );
    }
    let mut too_many = snapshot_tags(channel);
    too_many.push(tag("L", "nip-cl"));
    too_many.extend((0..33).map(|i| l(&format!("label:{i:02}"))));
    assert!(verify_snapshot(&event(39000, too_many, &keys), keys.public_key(), channel).is_err());
}
