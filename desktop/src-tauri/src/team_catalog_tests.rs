use super::*;
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};

fn event(keys: &Keys, created_at: u64, d_tag: &str, shared: bool, content: Value) -> Event {
    let mut tags = vec![Tag::parse(["d", d_tag]).unwrap()];
    if shared {
        tags.push(Tag::parse(["shared", "true"]).unwrap());
    }
    EventBuilder::new(Kind::Custom(KIND_TEAM_CATALOG as u16), content.to_string())
        .tags(tags)
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .unwrap()
}

fn valid_content(name: &str) -> Value {
    json!({
        "v": 1,
        "name": name,
        "description": "A crew.",
        "instructions": "Ship it.",
        "members": [{
            "member_key": "a".repeat(64),
            "display_name": "Reviewer",
            "system_prompt": "Review changes.",
            "avatar_url": "https://relay.example/avatar.png",
            "runtime": "goose",
            "model": "claude",
            "name_pool": ["Reviewer"],
            "respond_to": "owner-only",
            "parallelism": 4
        }]
    })
}

#[test]
fn paging_uses_oldest_verified_cursor_and_stops_on_ties_or_short_pages() {
    let keys = Keys::generate();
    let newest = event(&keys, 9, "newest", true, valid_content("Newest"));
    let oldest = event(&keys, 4, "oldest", true, valid_content("Oldest"));
    let mut by_id = HashMap::new();

    assert_eq!(
        merge_verified_page(
            &mut by_id,
            CATALOG_PAGE_SIZE,
            vec![newest.clone(), oldest.clone()]
        ),
        PageProgress::Next(4)
    );
    assert_eq!(
        merge_verified_page(&mut by_id, CATALOG_PAGE_SIZE, vec![newest, oldest]),
        PageProgress::Done
    );

    let short = event(&keys, 1, "short", true, valid_content("Short"));
    assert_eq!(
        merge_verified_page(&mut by_id, CATALOG_PAGE_SIZE - 1, vec![short]),
        PageProgress::Done
    );
    assert_eq!(
        merge_verified_page(&mut HashMap::new(), CATALOG_PAGE_SIZE, Vec::new()),
        PageProgress::Done
    );
}

#[test]
fn forged_newest_head_is_dropped_before_it_can_claim_the_coordinate() {
    let keys = Keys::generate();
    let older = event(&keys, 1, "crew", true, valid_content("Older"));
    let mut forged = event(&keys, 2, "crew", true, valid_content("Forged"));
    forged.content = valid_content("Tampered").to_string();

    let verified = [older.clone(), forged]
        .into_iter()
        .filter(|candidate| candidate.verify().is_ok())
        .collect();
    let publications = publications_from_verified_events(verified);
    assert_eq!(publications.len(), 1);
    assert_eq!(publications[0].event_id, older.id.to_hex());
    assert_eq!(publications[0].name, "Older");
}

#[test]
fn valid_newest_head_claims_before_visibility_and_content_parsing() {
    let keys = Keys::generate();
    for newest in [
        event(&keys, 2, "crew", false, valid_content("Unshared")),
        event(&keys, 2, "crew", true, json!({"v": 1})),
    ] {
        let older = event(&keys, 1, "crew", true, valid_content("Older"));
        assert!(publications_from_verified_events(vec![older, newest]).is_empty());
    }
}

#[test]
fn equal_heads_use_lowest_event_id_and_authors_are_independent() {
    let alice = Keys::generate();
    let bob = Keys::generate();
    let shared = event(&alice, 1, "crew", true, valid_content("Shared"));
    let unshared = event(&alice, 1, "crew", false, valid_content("Hidden"));
    let bob_head = event(&bob, 1, "crew", true, valid_content("Bob"));
    let expected_alice = if shared.id < unshared.id { 1 } else { 0 };

    let publications = publications_from_verified_events(vec![shared, unshared, bob_head]);
    assert_eq!(publications.len(), expected_alice + 1);
}

#[test]
fn all_or_nothing_parse_drops_a_team_with_any_invalid_member() {
    let keys = Keys::generate();
    // parallelism 999 is out of the 1..=32 range validate_member enforces, so
    // the whole projection fails to parse and the team is not offered.
    let mut invalid = valid_content("Broken");
    invalid["members"][0]["parallelism"] = json!(999);
    let head = event(&keys, 1, "crew", true, invalid);
    assert!(publications_from_verified_events(vec![head]).is_empty());
}

#[test]
fn projection_flattens_members_and_defaults_absent_system_prompt() {
    let keys = Keys::generate();
    let mut content = valid_content("Crew");
    // A member whose system_prompt is absent must project as an empty string,
    // not be dropped — mirrors the renderer's `?? ""`.
    content["members"][0]
        .as_object_mut()
        .unwrap()
        .remove("system_prompt");
    let head = event(&keys, 1, "crew", true, content);

    let publications = publications_from_verified_events(vec![head]);
    assert_eq!(publications.len(), 1);
    let member = &publications[0].members[0];
    assert_eq!(member.display_name, "Reviewer");
    assert_eq!(member.system_prompt, "");
    assert_eq!(member.model.as_deref(), Some("claude"));
}

#[test]
fn multi_d_and_empty_d_heads_are_rejected() {
    let keys = Keys::generate();
    let empty_d = event(&keys, 1, "", true, valid_content("Empty"));
    assert!(publications_from_verified_events(vec![empty_d]).is_empty());

    let multi_d = EventBuilder::new(
        Kind::Custom(KIND_TEAM_CATALOG as u16),
        valid_content("Multi").to_string(),
    )
    .tags([
        Tag::parse(["d", "crew"]).unwrap(),
        Tag::parse(["d", "other"]).unwrap(),
        Tag::parse(["shared", "true"]).unwrap(),
    ])
    .custom_created_at(Timestamp::from(1))
    .sign_with_keys(&keys)
    .unwrap();
    assert!(publications_from_verified_events(vec![multi_d]).is_empty());
}

/// Pins the serialized DTO output against the renderer's catalog contract.
/// The Tauri generic is only a TypeScript assertion; serde's bytes are the
/// actual boundary, so compare the value with an absent optional field.
#[test]
fn serialized_catalog_matches_the_typescript_contract() {
    let publication = TeamCatalogPublication {
        event_id: "ev1".into(),
        owner_pubkey: "owner".into(),
        team_d_tag: "team-1".into(),
        name: "Crew".into(),
        description: Some("A crew.".into()),
        instructions: None,
        members: vec![TeamCatalogMemberProjection {
            member_key: "k1".into(),
            display_name: "Ada".into(),
            system_prompt: "be kind".into(),
            avatar_url: Some("https://example.com/a.png".into()),
            runtime: Some("acp".into()),
            model: None,
            provider: Some("p1".into()),
        }],
    };
    let actual = serde_json::to_value(vec![publication]).unwrap();
    let expected = serde_json::json!([{
        "eventId": "ev1",
        "ownerPubkey": "owner",
        "teamDTag": "team-1",
        "name": "Crew",
        "description": "A crew.",
        "members": [{
            "memberKey": "k1",
            "displayName": "Ada",
            "systemPrompt": "be kind",
            "avatarUrl": "https://example.com/a.png",
            "runtime": "acp",
            "model": null,
            "provider": "p1",
        }],
    }]);
    assert_eq!(actual, expected);
}
