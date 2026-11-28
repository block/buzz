use super::*;
use nostr::{Keys, Timestamp};

const FLOOR: &str =
    r#"{"version":1,"body":{"type":"floor","rules":[],"labels":{},"max_grant_seconds":3600}}"#;

fn signed(tags: Vec<Tag>, content: &str) -> Event {
    EventBuilder::new(Kind::Custom(KIND_GOVERNANCE_COMMAND as u16), content)
        .tags(tags)
        .custom_created_at(Timestamp::from(100))
        .sign_with_keys(&Keys::generate())
        .unwrap()
}

fn tags() -> Vec<Tag> {
    [
        ("community", "00000000-0000-0000-0000-000000000001"),
        ("d", "floor"),
        ("prev", "absent"),
    ]
    .into_iter()
    .map(|(k, v)| Tag::parse([k, v]).unwrap())
    .collect()
}

#[test]
fn command_unbound() {
    assert!(Command::parse(&signed(tags(), FLOOR)).is_ok());
    for name in ["community", "d", "prev"] {
        let original = tags();
        let missing = original
            .iter()
            .filter(|t| t.as_slice()[0] != name)
            .cloned()
            .collect();
        assert!(
            Command::parse(&signed(missing, FLOOR)).is_err(),
            "missing {name}"
        );
        let mut duplicate = original.clone();
        duplicate.push(
            original
                .iter()
                .find(|t| t.as_slice()[0] == name)
                .unwrap()
                .clone(),
        );
        assert!(
            Command::parse(&signed(duplicate, FLOOR)).is_err(),
            "duplicate {name}"
        );
        let mut malformed = original;
        malformed.retain(|t| t.as_slice()[0] != name);
        malformed.push(Tag::parse([name, "floor", "extra"]).unwrap());
        assert!(
            Command::parse(&signed(malformed, FLOOR)).is_err(),
            "malformed {name}"
        );
    }
}

#[test]
fn command_signature_covers_body_and_address() {
    let mut event = signed(tags(), FLOOR);
    event.content = FLOOR.replace("3600", "3601");
    assert!(Command::parse(&event).is_err());
    let mut wrong = tags();
    wrong.retain(|t| t.as_slice()[0] != "d");
    wrong.push(Tag::parse(["d", "trusted"]).unwrap());
    assert!(Command::parse(&signed(wrong, FLOOR)).is_err());
}

#[test]
fn unknown_and_ambiguous_document_semantics_are_rejected() {
    for content in [
        FLOOR.replace("\"version\":1", "\"version\":2"),
        FLOOR.replace("\"version\":1", "\"version\":1,\"version\":1"),
        FLOOR.replace("\"labels\":{}", "\"labels\":{},\"labels\":{}"),
        FLOOR.replace("\"rules\":[]", "\"rules\":[],\"bypass\":true"),
        FLOOR.replace("\"floor\"", "\"future_floor\""),
        format!("{FLOOR} {FLOOR}"),
    ] {
        assert!(
            Document::parse(&content, &DocumentId::Floor).is_err(),
            "{content}"
        );
    }
}

#[test]
fn rules_keep_freshness_explicit_and_reject_unknown_requirements() {
    let id: DocumentId = format!("rules:30617:{}:repo", "a".repeat(64))
        .parse()
        .unwrap();
    for fresh in [true, false] {
        let value = serde_json::json!({"version":1,"body":{"type":"rules","rules":[{"id":"review","refs":["refs/heads/**"],"labels":{},"require":[{"type":"review","count":2,"group":null},{"type":"fresh_target","required":fresh}]}]}});
        assert!(Document::parse(&value.to_string(), &id).is_ok());
        // Change the typed discriminator independently of JSON field ordering.
        let mut bad_value = value;
        bad_value["body"]["rules"][0]["require"][0]["type"] = "future_review".into();
        assert!(Document::parse(&bad_value.to_string(), &id).is_err());
    }
}

#[test]
fn repository_coordinates_match_safe_repository_names() {
    for name in ["repo", "a.b", "a-b_1"] {
        assert!(format!("30617:{}:{name}", "a".repeat(64))
            .parse::<RepoCoordinate>()
            .is_ok());
    }
    for name in ["", ".hidden", "a..b", "../x", "a:b", "a/b"] {
        assert!(format!("30617:{}:{name}", "a".repeat(64))
            .parse::<RepoCoordinate>()
            .is_err());
    }
}

#[test]
fn commands_cannot_author_unsupported_requirements() {
    let id = format!("rules:30617:{}:repo", "a".repeat(64));
    let rules = |req: serde_json::Value| {
        serde_json::json!({"version":1,"body":{"type":"rules","rules":[{"id":"r","refs":["refs/heads/**"],"labels":{},"require":[req]}]}}).to_string()
    };
    let with = |content: &str| {
        let mut t = tags();
        t[1] = Tag::parse(["d", &id]).unwrap();
        signed(t, content)
    };
    for (req, reason) in [
        (serde_json::json!({"type":"codeowners"}), "codeowners"),
        (
            serde_json::json!({"type":"review","count":1,"group":"00000000-0000-0000-0000-000000000002"}),
            "group review",
        ),
    ] {
        let content = rules(req);
        // A stored head that names one still reads, and denies at the land.
        assert!(Document::parse(&content, &id.parse().unwrap()).is_ok());
        let refused = Command::parse(&with(&content)).unwrap_err();
        assert!(refused.contains(reason), "{refused}");
        assert!(command_builder(
            Uuid::nil(),
            &id.parse().unwrap(),
            &ExpectedHead::Absent,
            &content
        )
        .is_err());
    }
    let plain = rules(serde_json::json!({"type":"review","count":1,"group":null}));
    assert!(Command::parse(&with(&plain)).is_ok());
}
