//! Adoption-path concealment gate (Carl P1): a signed, shared, current head
//! carrying a bidi override in executable text must be refused before adoption
//! writes anything — no persona copy, no team record, no retention row.
//!
//! Driven through the highest in-process seam: the `add_verified_team` body
//! with the relay fetch elided. `verified_head_content` is exactly what
//! `verified_catalog_head` runs on the fetched head (`adopt.rs:121`), and
//! `commit_and_enqueue` against real temp stores plus a real retention scope is
//! the production write path (`adopt.rs:132`). Data flow forces
//! validate-before-write: the commit consumes the plan, the plan consumes the
//! parsed content, so a write cannot precede the gate without stubbing it.

use super::super::apply::{commit_and_enqueue, plan_add};
use super::super::verified_head_content;
use super::*;
use crate::managed_agents::retention::{scoped_retention_db_path, RetentionScope};
use nostr::{EventBuilder, Kind, Tag};

const RELAY: &str = "wss://relay.example";

/// A retention scope rooted in a fresh temp dir. The db file is created only
/// when a row is enqueued, so its absence proves nothing was retained.
fn scope(dir: &std::path::Path) -> RetentionScope {
    let keys = nostr::Keys::generate();
    let db_path = scoped_retention_db_path(dir, RELAY, &keys.public_key().to_hex());
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    RetentionScope {
        db_path,
        relay_url: RELAY.to_string(),
        owner_keys: keys,
    }
}

/// The externally-requested contract: a signed, shared, current head carrying
/// concealed executable text is refused, and adoption leaves the personas
/// store, the teams store, and retention untouched. Goes RED if the concealment
/// gate is removed — the parse then succeeds, the commit writes both stores, and
/// the enqueue creates a retention db.
#[test]
fn a_concealed_head_is_refused_and_writes_no_store_or_retention_row() {
    let dir = tempfile::tempdir().unwrap();
    let personas_path = dir.path().join("personas.json");
    let teams_path = dir.path().join("teams.json");
    std::fs::write(&personas_path, b"[]").unwrap();
    std::fs::write(&teams_path, b"[]").unwrap();
    let scope = scope(dir.path());

    let keys = nostr::Keys::generate();
    let mut concealed = member("m1", "Run\u{2066}hidden");
    concealed.display_name = "One".to_string();
    let body = serde_json::to_string(&content(vec![concealed])).unwrap();
    let event = EventBuilder::new(Kind::Custom(30178), body)
        .tags(vec![
            Tag::parse(["d", TEAM_D_TAG]).unwrap(),
            Tag::parse(["shared", "true"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let source = source(&keys.public_key().to_hex());

    // The add_verified_team sequence: verify+parse (the gate), then plan, then
    // the real store write and retention enqueue. The write closure and scope
    // resolver run only if the gate lets the content through.
    let resolved = RetentionScope {
        db_path: scope.db_path.clone(),
        relay_url: scope.relay_url.clone(),
        owner_keys: scope.owner_keys.clone(),
    };
    let result = (|| {
        let content = verified_head_content(&event, &source, &event.id.to_hex())?;
        let plan = plan_add(&[], &[], &source, &content, NOW)?;
        commit_and_enqueue(
            plan,
            |personas, teams| {
                std::fs::write(&personas_path, serde_json::to_vec(personas).unwrap())
                    .map_err(|e| e.to_string())?;
                std::fs::write(&teams_path, serde_json::to_vec(teams).unwrap())
                    .map_err(|e| e.to_string())?;
                Ok(())
            },
            || Ok(resolved),
        )
    })();

    let error = result.expect_err("a concealed head must be rejected");
    assert!(
        error.contains("prohibited invisible or formatting character"),
        "the rejection must name the concealment rule: {error}"
    );
    assert_eq!(
        std::fs::read(&personas_path).unwrap(),
        b"[]",
        "the personas store must be byte-unchanged on a rejected adoption"
    );
    assert_eq!(
        std::fs::read(&teams_path).unwrap(),
        b"[]",
        "the teams store must be byte-unchanged on a rejected adoption"
    );
    assert!(
        !scope.db_path.exists(),
        "no retention db is created — a rejected adoption enqueues nothing"
    );
}
