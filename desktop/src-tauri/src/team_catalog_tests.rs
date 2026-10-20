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

#[tokio::test]
async fn full_advancing_pages_through_the_cap_error_rather_than_truncate() {
    // A catalog larger than MAX_CATALOG_PAGES can walk: every page is full and
    // advances the cursor, but the loop runs out of budget before a short page
    // proves exhaustion. Returning the collected heads would silently present a
    // truncated catalog as complete, so `collect_verified_catalog` must fail
    // loudly. Each fetched page carries a fresh valid event whose timestamp
    // strictly decreases, so the cursor keeps advancing (never DenseBoundary).
    let keys = Keys::generate();
    let mut fetches = 0usize;
    let result = collect_verified_catalog(|_until| {
        fetches += 1;
        // A newer-than-any-cursor timestamp per page, strictly descending so the
        // oldest verified timestamp always drops the inclusive `until`.
        let created_at = (MAX_CATALOG_PAGES - fetches + 1) as u64;
        let filler = event(
            &keys,
            created_at,
            &format!("team-{fetches}"),
            true,
            valid_content("T"),
        );
        async move { Ok((CATALOG_PAGE_SIZE, vec![filler])) }
    })
    .await;

    assert_eq!(
        fetches, MAX_CATALOG_PAGES,
        "the full page budget is consumed"
    );
    let error = result.expect_err("a catalog that never ends must not return Ok");
    assert!(
        error.contains("page fetch budget"),
        "truncation is reported as a loud error, got: {error}"
    );
}

#[tokio::test]
async fn short_page_on_the_final_allowed_page_completes_ok() {
    // The cap boundary must not be off-by-one: a short page delivered on the
    // very last allowed page proves exhaustion and completes Ok. Every prior
    // page is full and advancing; the final page is short.
    let keys = Keys::generate();
    let mut fetches = 0usize;
    let result = collect_verified_catalog(|_until| {
        fetches += 1;
        let created_at = (MAX_CATALOG_PAGES - fetches + 1) as u64;
        let d_tag = format!("team-{fetches}");
        let ev = event(&keys, created_at, &d_tag, true, valid_content("T"));
        // Full pages until the last allowed one, which is short → Done.
        let page_len = if fetches < MAX_CATALOG_PAGES {
            CATALOG_PAGE_SIZE
        } else {
            CATALOG_PAGE_SIZE - 1
        };
        async move { Ok((page_len, vec![ev])) }
    })
    .await;

    assert_eq!(
        fetches, MAX_CATALOG_PAGES,
        "paging reaches the final allowed page"
    );
    let by_id = result.expect("a short final page proves exhaustion and completes");
    assert_eq!(
        by_id.len(),
        MAX_CATALOG_PAGES,
        "every page's event is collected"
    );
}

#[test]
fn paging_advances_on_verified_oldest_and_stops_on_short_pages() {
    let keys = Keys::generate();
    let newest = event(&keys, 9, "newest", true, valid_content("Newest"));
    let oldest = event(&keys, 4, "oldest", true, valid_content("Oldest"));
    let mut by_id = HashMap::new();

    // First full page: no cursor yet, so the oldest verified timestamp (4)
    // becomes the next inclusive `until`.
    assert_eq!(
        merge_verified_page(
            &mut by_id,
            CATALOG_PAGE_SIZE,
            None,
            vec![newest.clone(), oldest.clone()]
        ),
        PageProgress::Next(4)
    );

    // A short page ends the catalog regardless of its timestamps.
    let short = event(&keys, 1, "short", true, valid_content("Short"));
    assert_eq!(
        merge_verified_page(&mut by_id, CATALOG_PAGE_SIZE - 1, Some(4), vec![short]),
        PageProgress::Done
    );

    // A short page with no verified events is still the end of the catalog:
    // NoVerifiedEvents only fires on a *full* page.
    assert_eq!(
        merge_verified_page(
            &mut HashMap::new(),
            CATALOG_PAGE_SIZE - 1,
            Some(4),
            Vec::new()
        ),
        PageProgress::Done
    );
}

#[test]
fn full_page_stuck_at_boundary_second_reports_dense_not_done() {
    // Regression for Carl blocker 2: a full page whose oldest verified timestamp
    // ties the inclusive `until` cursor cannot be paged past (the relay filter
    // has no sub-second cursor). It must report DenseBoundary, not silently
    // complete and drop every older team.
    let keys = Keys::generate();
    let a = event(&keys, 7, "a", true, valid_content("A"));
    let b = event(&keys, 7, "b", true, valid_content("B"));
    let mut by_id = HashMap::new();

    // Under a cursor of 7, a full page whose oldest is also 7 is dense.
    assert_eq!(
        merge_verified_page(&mut by_id, CATALOG_PAGE_SIZE, Some(7), vec![a, b]),
        PageProgress::DenseBoundary(7)
    );
}

#[test]
fn mixed_page_advances_on_verified_oldest_ignoring_older_unverifiable_event() {
    // An attacker-controlled relay page can carry a forged event with
    // `created_at = 0` alongside genuinely newer valid teams. Drive the exact
    // production trust gate: the raw page `[valid@9, valid@4, forged@0]` goes
    // through `verify_page` (the same helper `fetch_team_catalog` calls), which
    // drops the tampered event before it can reach paging. The cursor then
    // advances on the oldest *verified* timestamp (4), never the forged wire
    // timestamp (0) — advancing to 0 would skip every valid team between the
    // verified floor and zero. Constructing the forged event here rather than
    // stubbing `verify_page`'s output means a desync of the raw-page
    // verification/cursor plumbing would fail this test.
    let keys = Keys::generate();
    let newest = event(&keys, 9, "newest", true, valid_content("Newest"));
    let oldest = event(&keys, 4, "oldest", true, valid_content("Oldest"));
    // Sign at 0, then tamper the content so the signature no longer matches.
    let mut forged = event(&keys, 0, "forged", true, valid_content("Forged"));
    forged.content = valid_content("Tampered").to_string();

    let verified = verify_page(vec![newest.clone(), oldest.clone(), forged]);
    // The forged event is gone; only the two genuinely signed events survive.
    assert_eq!(verified.len(), 2);

    let mut by_id = HashMap::new();
    assert_eq!(
        merge_verified_page(&mut by_id, CATALOG_PAGE_SIZE, None, verified),
        PageProgress::Next(4)
    );
}

#[test]
fn full_page_of_unverifiable_events_errors_rather_than_advancing() {
    // A full wire page whose events all fail verification leaves the verified
    // set empty. The cursor can only move on trusted timestamps, so this must
    // report NoVerifiedEvents (a loud error at the call site), never Done or
    // Next — advancing on the untrusted wire would let a forged `created_at`
    // silently drop every valid team below it. Drive the real `verify_page`
    // seam: a tampered event at `created_at = 0` is dropped, leaving nothing to
    // page with even though the wire page was full.
    let keys = Keys::generate();
    let mut forged = event(&keys, 0, "forged", true, valid_content("Forged"));
    forged.content = valid_content("Tampered").to_string();

    let verified = verify_page(vec![forged]);
    assert!(verified.is_empty());

    let mut by_id = HashMap::new();
    assert_eq!(
        merge_verified_page(&mut by_id, CATALOG_PAGE_SIZE, Some(9), verified),
        PageProgress::NoVerifiedEvents
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
