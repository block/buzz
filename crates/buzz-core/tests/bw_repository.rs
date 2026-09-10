use buzz_core::bw::{parse_json, Consumer, Decision};
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn input() -> Value {
    let bytes = include_bytes!("data/bw_owner_genesis.json");
    assert_eq!(
        hex::encode(Sha256::digest(bytes)),
        "46183ef1eaa2ce01267cc185d53d0235b87f821ed313233789a5d3cb4ce67a67"
    );
    parse_json(bytes).expect("unchanged public signed input")
}

fn consumer(v: &Value) -> Consumer {
    Consumer::new(
        serde_json::from_value(v["trust"].clone()).expect("trust"),
        serde_json::from_value(v["external"].clone()).expect("external"),
        v["now"].as_u64().expect("now"),
    )
}

fn feed(c: &mut Consumer, event: &Value) -> Decision {
    c.ingest(&serde_json::to_vec(event).expect("wire"))
}

#[test]
fn unchanged_owner_announcement_and_genesis_are_accepted() {
    let v = input();
    let mut c = consumer(&v);
    for event in v["events"].as_array().expect("events") {
        let out = feed(&mut c, event);
        assert_eq!(out.event_id.as_deref(), event["id"].as_str());
        assert_eq!(
            (out.outcome.as_str(), out.stage.as_str(), out.code.as_str()),
            ("accept", "projection", "valid"),
            "{out:?}"
        );
        assert_eq!(out.projection["dispatch_count"], 0);
        assert_eq!(out.projection["issues"], json!({}));
    }
}

#[test]
fn genuine_signed_events_still_require_the_correct_id_and_signature() {
    let v = input();
    for event in v["events"].as_array().expect("events") {
        for (field, bad, stage, code) in [
            ("id", "0".repeat(64), "id", "event-id"),
            ("sig", "0".repeat(128), "signature", "signature"),
        ] {
            let mut corrupted = event.clone();
            corrupted[field] = json!(bad);
            let out = feed(&mut consumer(&v), &corrupted);
            assert_eq!(
                (out.outcome.as_str(), out.stage.as_str(), out.code.as_str()),
                ("reject", stage, code)
            );
        }
    }
}

#[test]
fn unchanged_announcement_cannot_establish_foreign_repository_trust() {
    let original = input();
    for owner in [false, true] {
        let mut v = original.clone();
        if owner {
            // Public P1 fictitious owner (scalar 1), never a production key.
            let public = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
            v["trust"]["owner"] = json!(public);
            v["trust"]["repo"] = json!(format!("30617:{public}:mybuzz"));
        } else {
            v["trust"]["repo"] = json!(format!(
                "{}-other",
                v["trust"]["repo"].as_str().expect("repo")
            ));
        }
        let out = feed(&mut consumer(&v), &original["events"][0]);
        assert_eq!(
            (out.outcome.as_str(), out.stage.as_str(), out.code.as_str()),
            ("reject", "references", "repository")
        );
    }
}

// Mutated shape inputs are freshly signed with the existing public P1 scalar 1.
// The real owner announcement and genesis above are never changed or re-signed.
fn signed(event: &Value, tags: Value) -> Value {
    let mut scalar = [0u8; 32];
    scalar[31] = 1;
    let keys = Keys::parse(&hex::encode(scalar)).expect("public fictitious key");
    let tags = tags
        .as_array()
        .expect("tags")
        .iter()
        .map(|t| {
            Tag::parse(
                t.as_array()
                    .expect("tag")
                    .iter()
                    .map(|v| v.as_str().expect("string")),
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .expect("tags");
    let event = EventBuilder::new(
        Kind::Custom(event["kind"].as_u64().expect("kind") as u16),
        event["content"].as_str().expect("content"),
    )
    .tags(tags)
    .custom_created_at(Timestamp::from(event["created_at"].as_u64().expect("time")))
    .sign_with_keys(&keys)
    .expect("sign fictitious event");
    serde_json::from_str(&event.as_json()).expect("wire")
}

#[test]
fn repository_channel_is_singleton_and_does_not_relax_bw_tags() {
    let d = parse_json(include_bytes!("../../../docs/nips/NIP-BW.fixtures.json")).expect("corpus");
    let case = &d["cases"][0];
    let repo = &d["events"]["repo"]["event"];
    for (extra, code) in [
        (json!([["buzz-channel"]]), "arity"),
        (json!([["buzz-channel", "channel", "extra"]]), "arity"),
        (
            json!([["buzz-channel", "one"], ["buzz-channel", "two"]]),
            "duplicate-tag",
        ),
        (json!([["buzz-unknown", "value"]]), "unknown-tag"),
    ] {
        let mut tags = repo["tags"].clone();
        tags.as_array_mut()
            .expect("tags")
            .extend(extra.as_array().expect("extra").iter().cloned());
        let out = feed(&mut consumer(case), &signed(repo, tags));
        assert_eq!(
            (out.outcome.as_str(), out.stage.as_str(), out.code.as_str()),
            ("reject", "shape", code),
            "{out:?}"
        );
    }
    let policy = &d["events"]["policy"]["event"];
    let mut tags = policy["tags"].clone();
    tags.as_array_mut()
        .expect("tags")
        .push(json!(["buzz-channel", "channel"]));
    let out = feed(&mut consumer(case), &signed(policy, tags));
    assert_eq!(
        (out.outcome.as_str(), out.stage.as_str(), out.code.as_str()),
        ("reject", "shape", "unknown-tag")
    );
}

#[test]
fn existing_nip34_multivalue_metadata_remains_supported() {
    let d = parse_json(include_bytes!("../../../docs/nips/NIP-BW.fixtures.json")).expect("corpus");
    let repo = &d["events"]["repo"]["event"];
    let mut tags = repo["tags"].clone();
    tags.as_array_mut().expect("tags").extend([
        json!(["buzz-channel", "f628519d-147b-4d1f-b21a-fb3c557d3aa5"]),
        json!([
            "clone",
            "https://example.invalid/repo.git",
            "git@example.invalid:repo.git"
        ]),
        json!([
            "web",
            "https://example.invalid/repo",
            "https://example.invalid/mirror"
        ]),
        json!(["relays", "wss://example.invalid", "ws://localhost:3000"]),
        json!([
            "maintainers",
            repo["pubkey"],
            "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"
        ]),
        json!(["r", "1".repeat(40), "euc"]),
        json!(["t", "rust"]),
    ]);
    let out = feed(&mut consumer(&d["cases"][0]), &signed(repo, tags));
    assert_eq!(out.outcome, "accept", "{out:?}");
}
