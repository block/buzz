use super::*;
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::json;

fn event(keys: &Keys, created_at: u64, source: &str, shared: bool, content: Value) -> Event {
    let mut tags = vec![Tag::parse(["d", source]).unwrap()];
    if shared {
        tags.push(Tag::parse(["shared", "true"]).unwrap());
    }
    EventBuilder::new(Kind::Custom(KIND_PERSONA as u16), content.to_string())
        .tags(tags)
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .unwrap()
}

fn valid_content(name: &str) -> Value {
    json!({
        "display_name": name,
        "system_prompt": "Review changes.",
        "avatar_url": "https://relay.example/avatar.png",
        "runtime": " goose ",
        "model": "claude",
        "provider": null,
        "name_pool": ["Reviewer", 7],
        "respond_to": "allowlist",
        "parallelism": 4
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
    let older = event(&keys, 1, "reviewer", true, valid_content("Older"));
    let mut forged = event(&keys, 2, "reviewer", true, valid_content("Forged"));
    forged.content = valid_content("Tampered").to_string();

    let verified = [older.clone(), forged]
        .into_iter()
        .filter(|candidate| candidate.verify().is_ok())
        .collect();
    let publications = publications_from_verified_events(verified);
    assert_eq!(publications.len(), 1);
    assert_eq!(publications[0].event_id, older.id.to_hex());
}

#[test]
fn valid_newest_head_claims_before_visibility_and_content_parsing() {
    let keys = Keys::generate();
    for newest in [
        event(&keys, 2, "reviewer", false, valid_content("Unshared")),
        event(&keys, 2, "reviewer", true, json!({})),
    ] {
        let older = event(&keys, 1, "reviewer", true, valid_content("Older"));
        assert!(publications_from_verified_events(vec![older, newest]).is_empty());
    }
}

#[test]
fn equal_second_heads_use_lowest_event_id_and_authors_are_independent() {
    let alice = Keys::generate();
    let bob = Keys::generate();
    let shared = event(&alice, 1, "reviewer", true, valid_content("Shared"));
    let unshared = event(&alice, 1, "reviewer", false, valid_content("Hidden"));
    let bob_head = event(&bob, 1, "reviewer", true, valid_content("Bob"));
    let expected_alice = if shared.id < unshared.id { 1 } else { 0 };

    let publications = publications_from_verified_events(vec![shared, unshared, bob_head]);
    assert_eq!(publications.len(), expected_alice + 1);
}

#[test]
fn parser_projects_types_and_foreign_allowlists_exactly() {
    let projection = parse_agent(&valid_content("Reviewer").to_string()).unwrap();
    assert_eq!(projection.display_name, "Reviewer");
    assert_eq!(projection.runtime.as_deref(), Some(" goose "));
    assert_eq!(projection.provider, None);
    assert_eq!(projection.name_pool, vec!["Reviewer"]);
    assert_eq!(projection.respond_to.as_deref(), Some("owner-only"));
    assert_eq!(projection.parallelism, Some(4));

    for bad in [0, 33] {
        let mut content = valid_content("Reviewer");
        content["parallelism"] = json!(bad);
        assert_eq!(parse_agent(&content.to_string()).unwrap().parallelism, None);
    }
}

#[test]
fn parser_rejects_malformed_and_invisible_definition_text() {
    for content in [
        "not-json".to_string(),
        "[]".to_string(),
        json!({"display_name": 7}).to_string(),
        valid_content("Review\u{202e}er").to_string(),
    ] {
        assert!(parse_agent(&content).is_none());
    }
    // A description that violates the shared visible-text policy or the
    // 280-char cap rejects the whole entry — never silently stripped.
    for bad_description in [
        "hidden\u{200b}text".to_string(),
        "description\n".to_string(),
        "a".repeat(281),
    ] {
        let mut content = valid_content("Reviewer");
        content["description"] = json!(bad_description);
        assert!(parse_agent(&content.to_string()).is_none());
    }
    for malformed_description in [json!(7), json!([]), json!({})] {
        let mut content = valid_content("Reviewer");
        content["description"] = malformed_description;
        assert!(parse_agent(&content.to_string()).is_none());
    }
    let mut content = valid_content("Reviewer");
    content["description"] = json!("A careful reviewer.");
    assert_eq!(
        parse_agent(&content.to_string())
            .unwrap()
            .description
            .as_deref(),
        Some("A careful reviewer.")
    );
    let visible = parse_agent(
        &json!({
            "display_name": "Reviewer 🐝",
            "system_prompt": "Review.\n\t||literal markdown||"
        })
        .to_string(),
    )
    .unwrap();
    assert_eq!(visible.display_name, "Reviewer 🐝");
}

#[test]
fn avatar_allowlist_and_bounds_match_the_renderer_contract() {
    assert!(safe_avatar("https://relay.example/avatar.png"));
    assert!(!safe_avatar("javascript:alert(1)"));
    assert!(safe_avatar("data:image/svg+xml,<svg/>"));
    assert!(!safe_avatar(&format!(
        "data:image/svg+xml,{}",
        "a".repeat(MAX_INLINE_SVG_AVATAR_LENGTH)
    )));
    for mime in ["png", "jpeg", "gif", "webp"] {
        assert!(safe_avatar(&format!(
            "data:image/{mime};base64,iVBORw0KGgo="
        )));
    }
    assert!(!safe_avatar("data:image/bmp;base64,aA=="));
    assert!(!safe_avatar("data:image/png;base64,not base64"));
}

#[test]
fn exact_tags_reject_duplicates_and_extra_fields() {
    let keys = Keys::generate();
    let base = event(&keys, 1, "reviewer", true, valid_content("Reviewer"));
    assert_eq!(exact_tag(&base, "shared").as_deref(), Some("true"));

    let duplicate = EventBuilder::new(
        Kind::Custom(KIND_PERSONA as u16),
        valid_content("x").to_string(),
    )
    .tags([
        Tag::parse(["d", "reviewer"]).unwrap(),
        Tag::parse(["shared", "true"]).unwrap(),
        Tag::parse(["shared", "true"]).unwrap(),
    ])
    .sign_with_keys(&keys)
    .unwrap();
    assert_eq!(exact_tag(&duplicate, "shared"), None);

    // The old renderer accepts an extended d tag (it reads tag[1]) but shared
    // is opt-in only for the exact two-field shape.
    let extended = EventBuilder::new(
        Kind::Custom(KIND_PERSONA as u16),
        valid_content("x").to_string(),
    )
    .tags([
        Tag::parse(["d", "reviewer", "relay hint"]).unwrap(),
        Tag::parse(["shared", "true", "extra"]).unwrap(),
    ])
    .sign_with_keys(&keys)
    .unwrap();
    assert_eq!(coordinate_tag(&extended, "d").as_deref(), Some("reviewer"));
    assert_eq!(exact_tag(&extended, "shared"), None);
}

/// Pins the serialized DTO output against the renderer's catalog contract.
/// The Tauri generic is only a TypeScript assertion; serde's bytes are the
/// actual boundary, so populate every optional field and compare the value.
#[test]
fn serialized_catalog_matches_the_typescript_contract() {
    let publication = PersonaCatalogPublication {
        event_id: "ev1".into(),
        owner_pubkey: "owner".into(),
        source_persona_id: "persona-1".into(),
        created_at: 42,
        agent: CatalogAgentProjection {
            display_name: "Ada".into(),
            avatar_url: Some("https://example.com/a.png".into()),
            description: Some("A kind agent.".into()),
            system_prompt: "be kind".into(),
            runtime: Some("acp".into()),
            model: Some("m1".into()),
            provider: Some("p1".into()),
            name_pool: vec!["Ada".into(), "Lin".into()],
            respond_to: Some("mentions".into()),
            parallelism: Some(2),
        },
    };
    let actual = serde_json::to_value(vec![publication]).unwrap();
    let expected = serde_json::json!([{
        "eventId": "ev1",
        "ownerPubkey": "owner",
        "sourcePersonaId": "persona-1",
        "createdAt": 42,
        "agent": {
            "displayName": "Ada",
            "avatarUrl": "https://example.com/a.png",
            "description": "A kind agent.",
            "systemPrompt": "be kind",
            "runtime": "acp",
            "model": "m1",
            "provider": "p1",
            "namePool": ["Ada", "Lin"],
            "respondTo": "mentions",
            "parallelism": 2,
        },
    }]);
    assert_eq!(actual, expected);
}
