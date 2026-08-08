use super::*;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn member(id: &str, display_name: &str) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        display_name: display_name.to_string(),
        avatar_url: None,
        system_prompt: "Do the work.".to_string(),
        runtime: Some("goose".to_string()),
        model: Some("claude-opus-4".to_string()),
        provider: Some("anthropic".to_string()),
        name_pool: vec!["Alpha".to_string()],
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        team_catalog_source: None,
        env_vars: BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2026-07-30T00:00:00Z".to_string(),
        updated_at: "2026-07-30T00:00:00Z".to_string(),
    }
}

fn team() -> TeamRecord {
    TeamRecord {
        id: "team-abc".to_string(),
        name: "Catalog Team".to_string(),
        description: Some("A shared team".to_string()),
        instructions: Some("Coordinate carefully.".to_string()),
        persona_ids: vec!["m1".to_string(), "m2".to_string()],
        is_builtin: false,
        shared: false,
        catalog_source: None,
        source_dir: Some(PathBuf::from("/local/only/path")),
        is_symlink: true,
        symlink_target: Some("/somewhere/private".to_string()),
        version: Some("1.0".to_string()),
        created_at: "2026-07-30T00:00:00Z".to_string(),
        updated_at: "2026-07-30T00:00:00Z".to_string(),
    }
}

#[test]
fn test_projection_omits_local_only_team_fields() {
    let content = build_team_catalog_content(&team(), &[member("m1", "One")]).unwrap();
    let json = team_catalog_content_json(&content).unwrap();

    assert!(json.contains("\"name\":\"Catalog Team\""));
    for local_only in [
        "source_dir",
        "is_symlink",
        "symlink_target",
        "is_builtin",
        "version",
        "created_at",
        "updated_at",
        "persona_ids",
    ] {
        assert!(
            !json.contains(local_only),
            "local-only field '{local_only}' must never be projected"
        );
    }
}

#[test]
fn test_projection_never_contains_a_source_allowlist_pubkey() {
    // Allowlist entries are real pubkeys the owner trusts — must not appear in the projection.
    const SECRET_PEER: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    let mut one = member("m1", "One");
    one.respond_to = Some(RespondTo::Allowlist.as_str().to_string());
    one.respond_to_allowlist = vec![SECRET_PEER.to_string()];
    one.env_vars
        .insert("API_TOKEN".to_string(), "super-secret".to_string());

    let content = build_team_catalog_content(&team(), &[one]).unwrap();
    let json = team_catalog_content_json(&content).unwrap();

    assert!(!json.contains(SECRET_PEER), "allowlist pubkey leaked");
    assert!(!json.contains("super-secret"), "env var value leaked");
    assert!(!json.contains("API_TOKEN"), "env var key leaked");
    assert!(!json.contains("respond_to_allowlist"));
}

#[test]
fn test_allowlist_mode_downgrades_to_owner_only_not_an_empty_allowlist() {
    // Must downgrade the mode itself, not empty the list — empty list reads as mode with no trust.
    let mut one = member("m1", "One");
    one.respond_to = Some(RespondTo::Allowlist.as_str().to_string());
    one.respond_to_allowlist = vec!["a".repeat(64)];

    let content = build_team_catalog_content(&team(), &[one]).unwrap();

    assert_eq!(
        content.members[0].respond_to.as_deref(),
        Some(RespondTo::OwnerOnly.as_str())
    );
}

#[test]
fn test_non_allowlist_respond_to_modes_are_projected_verbatim() {
    for mode in [RespondTo::OwnerOnly, RespondTo::Anyone] {
        let mut one = member("m1", "One");
        one.respond_to = Some(mode.as_str().to_string());
        let content = build_team_catalog_content(&team(), &[one]).unwrap();
        assert_eq!(
            content.members[0].respond_to.as_deref(),
            Some(mode.as_str())
        );
    }
}

#[test]
fn test_parallelism_is_clamped_into_the_supported_range() {
    for (input, expected) in [(0u32, 1u32), (1, 1), (32, 32), (9_999, 32)] {
        let mut one = member("m1", "One");
        one.parallelism = Some(input);
        let content = build_team_catalog_content(&team(), &[one]).unwrap();
        assert_eq!(content.members[0].parallelism, Some(expected));
    }
}

#[test]
fn test_members_resolve_in_team_membership_order() {
    let personas = vec![member("m2", "Two"), member("m1", "One")];

    let resolved = resolve_team_members(&team(), &personas).unwrap();

    let ids: Vec<&str> = resolved.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        ["m1", "m2"],
        "order is part of the canonical bytes, so it follows the team, not the store"
    );
}

#[test]
fn test_unresolvable_member_fails_resolution_rather_than_being_skipped() {
    let error = resolve_team_members(&team(), &[member("m1", "One")]).unwrap_err();

    assert!(error.contains("team member m2 not found"));
}

#[test]
fn test_rebuilding_an_unchanged_team_reproduces_identical_bytes() {
    // The freshness reconcile republishes on a byte mismatch.
    let members = [member("m1", "One"), member("m2", "Two")];
    let first = build_team_catalog_content(&team(), &members).unwrap();
    let second = build_team_catalog_content(&team(), &members).unwrap();

    assert_eq!(
        team_catalog_content_json(&first),
        team_catalog_content_json(&second)
    );
}

#[test]
fn test_member_order_is_part_of_the_canonical_bytes() {
    let forward = [member("m1", "One"), member("m2", "Two")];
    let reversed = [member("m2", "Two"), member("m1", "One")];

    let a = build_team_catalog_content(&team(), &forward).unwrap();
    let b = build_team_catalog_content(&team(), &reversed).unwrap();

    assert_ne!(team_catalog_content_json(&a), team_catalog_content_json(&b));
}

#[test]
fn test_editing_a_member_definition_changes_the_team_bytes() {
    let before = build_team_catalog_content(&team(), &[member("m1", "One")]).unwrap();
    let mut edited = member("m1", "One");
    edited.system_prompt = "Do the work differently.".to_string();
    let after = build_team_catalog_content(&team(), &[edited]).unwrap();

    assert_ne!(
        team_catalog_content_json(&before),
        team_catalog_content_json(&after)
    );
}

/// Real built-in record (avatar cleared — live built-ins ship ~170 KiB inline PNG).
fn builtin_record(id: &str) -> AgentDefinition {
    let mut record = crate::managed_agents::built_in_persona_definition(id, "2026-07-30T00:00:00Z")
        .unwrap_or_else(|| panic!("'{id}' is not a built-in persona"));
    record.avatar_url = None;
    record
}

#[test]
fn test_builtin_member_carries_slug_and_projection_hash() {
    let content = build_team_catalog_content(&team(), &[builtin_record("builtin:fizz")]).unwrap();
    let projected = &content.members[0];

    assert_eq!(projected.builtin_slug.as_deref(), Some("fizz"));
    assert!(projected.projection_hash.is_some());
}

#[test]
fn test_non_builtin_member_carries_no_reuse_hint() {
    let content = build_team_catalog_content(&team(), &[member("m1", "One")]).unwrap();

    assert_eq!(content.members[0].builtin_slug, None);
    assert_eq!(content.members[0].projection_hash, None);
}

#[test]
fn test_a_record_flagged_builtin_without_the_canonical_id_carries_no_hint() {
    // `is_builtin` alone is not the identity: a pack-installed or adopted copy has no cross-install slug.
    let mut impostor = member("m1", "One");
    impostor.is_builtin = true;

    let content = build_team_catalog_content(&team(), &[impostor]).unwrap();

    assert_eq!(content.members[0].builtin_slug, None);
    assert_eq!(content.members[0].projection_hash, None);
}

#[test]
fn test_reuse_hash_changes_when_the_builtin_definition_changes() {
    // Same slug, different definition — the recipient must detect it and fall back.
    let original = builtin_record("builtin:fizz");
    let mut changed = original.clone();
    changed.system_prompt = "Review differently.".to_string();

    let a = build_team_catalog_content(&team(), &[original]).unwrap();
    let b = build_team_catalog_content(&team(), &[changed]).unwrap();

    assert_eq!(
        a.members[0].builtin_slug, b.members[0].builtin_slug,
        "the slug is unchanged, which is exactly why the hash must differ"
    );
    assert_ne!(a.members[0].projection_hash, b.members[0].projection_hash);
}

#[test]
fn test_reuse_hash_excludes_the_hint_fields_so_a_recipient_can_recompute_it() {
    // The recipient hashes its own local copy — no cross-install slug is involved.
    let builtin = builtin_record("builtin:fizz");
    let recomputed = local_member_projection_hash(&builtin);
    let content = build_team_catalog_content(&team(), &[builtin]).unwrap();
    let projected = &content.members[0];
    assert_eq!(
        projected.projection_hash.as_deref(),
        Some(recomputed.as_str())
    );
    let mut hint_free = projected.clone();
    hint_free.builtin_slug = None;
    hint_free.projection_hash = None;
    assert_eq!(
        projected.projection_hash.as_deref(),
        Some(member_projection_hash(&hint_free).as_str())
    );
}

#[test]
fn test_member_count_at_the_limit_is_accepted_and_one_over_is_rejected() {
    let at_limit: Vec<AgentDefinition> = (0..MAX_MEMBERS)
        .map(|i| member(&format!("m{i}"), &format!("Member {i}")))
        .collect();
    assert!(build_team_catalog_content(&team(), &at_limit).is_ok());

    let mut over = at_limit;
    over.push(member("extra", "Extra"));
    let error = build_team_catalog_content(&team(), &over).unwrap_err();
    assert!(error.contains("team too large to share"), "{error}");
    assert!(error.contains("65 members"), "{error}");
}

#[test]
fn test_oversized_avatar_on_a_builtin_is_omitted_from_the_projection() {
    // Built-in avatars over the cap are silently omitted; recipient gets default.
    let mut one = member("m1", "Builtin Avatar Hog");
    one.is_builtin = true;
    one.id = "builtin:fizz".to_string(); // gives builtin_catalog_slug() a non-empty slug
    one.avatar_url = Some("d".repeat(MAX_AVATAR_URL_BYTES + 1));

    let content = build_team_catalog_content(&team(), &[one]).unwrap();

    assert_eq!(content.members.len(), 1);
    assert!(
        content.members[0].avatar_url.is_none(),
        "oversized built-in avatar must be omitted — not rejected — from the projection"
    );
}

#[test]
fn test_oversized_avatar_on_a_non_builtin_fails_the_size_contract() {
    // Non-raster oversized avatar (https URL) produces an error; owner can act on it.
    let mut one = member("m1", "Avatar Hog");
    one.avatar_url = Some(format!(
        "https://example.com/{}",
        "a".repeat(MAX_AVATAR_URL_BYTES)
    ));
    let error = build_team_catalog_content(&team(), &[one]).unwrap_err();
    assert!(
        error.contains("avatar") || error.contains("too large"),
        "non-builtin oversized avatar must name the field in the error: {error}"
    );
}

#[test]
fn test_avatar_exactly_at_the_limit_is_accepted() {
    // Safe https:// URL at exactly the 2 048-char cap must be accepted.
    let url = format!(
        "https://example.com/{}",
        "a".repeat(2_048 - "https://example.com/".len())
    );
    let mut one = member("m1", "One");
    one.avatar_url = Some(url);
    assert!(build_team_catalog_content(&team(), &[one]).is_ok());
}

#[test]
fn test_many_legal_members_still_reject_on_the_total_ceiling() {
    // All members individually within bounds, but together exceed the relay ingest ceiling.
    let members: Vec<AgentDefinition> = (0..MAX_MEMBERS)
        .map(|i| {
            let mut one = member(&format!("m{i}"), &format!("Member {i}"));
            one.system_prompt = "p".repeat(MAX_SYSTEM_PROMPT_BYTES);
            one
        })
        .collect();

    let error = build_team_catalog_content(&team(), &members).unwrap_err();

    assert!(error.contains("the projection is"), "{error}");
    assert!(
        !error.contains("members (limit"),
        "the per-field bounds all pass; the total is what rejects: {error}"
    );
}

#[test]
fn test_the_total_ceiling_stays_under_the_relay_ingest_limit() {
    // MAX_EVENT_CONTENT_BYTES = 256 KiB; an accepted projection must fit.
    const { assert!(MAX_TOTAL_BYTES < 256 * 1024) };
}

#[test]
fn test_oversized_team_text_fields_are_rejected() {
    for (label, subject) in [
        ("the team name", {
            let mut t = team();
            t.name = "n".repeat(MAX_NAME_BYTES + 1);
            t
        }),
        ("the team description", {
            let mut t = team();
            t.description = Some("d".repeat(MAX_TEXT_BYTES + 1));
            t
        }),
        ("the team instructions", {
            let mut t = team();
            t.instructions = Some("i".repeat(MAX_INSTRUCTIONS_BYTES + 1));
            t
        }),
    ] {
        let error = build_team_catalog_content(&subject, &[member("m1", "One")]).unwrap_err();
        assert!(error.contains(label), "expected '{label}' in: {error}");
    }
}

#[test]
fn test_oversized_name_pool_is_rejected() {
    let mut one = member("m1", "Pool Hog");
    one.name_pool = (0..=MAX_NAME_POOL_ENTRIES).map(|i| i.to_string()).collect();

    let error = build_team_catalog_content(&team(), &[one]).unwrap_err();

    assert!(error.contains("name-pool entries"), "{error}");
}

#[test]
fn test_an_empty_team_projects_successfully() {
    let content = build_team_catalog_content(&team(), &[]).unwrap();

    assert!(content.members.is_empty());
    // `members` is not `skip_serializing_if`, so an empty team is explicit
    // rather than indistinguishable from an omitted field.
    assert!(team_catalog_content_json(&content)
        .unwrap()
        .contains("\"members\":[]"));
}

#[test]
fn test_event_uses_kind_30178_and_the_team_id_as_its_d_tag() {
    let event = build_team_catalog_event(&team(), &[member("m1", "One")], false)
        .unwrap()
        .sign_with_keys(&nostr::Keys::generate())
        .unwrap();

    assert_eq!(event.kind.as_u16() as u32, KIND_TEAM_CATALOG);
    let d_tags: Vec<&str> = event
        .tags
        .iter()
        .filter_map(|tag| {
            let parts = tag.as_slice();
            (parts.first().map(String::as_str) == Some("d")).then(|| parts[1].as_str())
        })
        .collect();
    // The relay rejects anything but exactly one bounded `d` tag.
    assert_eq!(d_tags, vec!["team-abc"]);
}

#[test]
fn test_shared_tag_is_present_only_when_sharing() {
    for shared in [true, false] {
        let event = build_team_catalog_event(&team(), &[member("m1", "One")], shared)
            .unwrap()
            .sign_with_keys(&nostr::Keys::generate())
            .unwrap();

        assert_eq!(
            buzz_core_pkg::kind::event_is_shared(&event),
            shared,
            "the relay read gate keys off this tag"
        );
    }
}

#[test]
fn test_oversized_team_fails_before_an_event_is_ever_built() {
    // Pre-enqueue: no signed event exists to be durably queued. Uses total-size violation.
    let members: Vec<AgentDefinition> = (0..MAX_MEMBERS)
        .map(|i| {
            let mut one = member(&format!("m{i}"), &format!("Member {i}"));
            one.system_prompt = "p".repeat(MAX_SYSTEM_PROMPT_BYTES);
            one
        })
        .collect();

    assert!(build_team_catalog_event(&team(), &members, true).is_err());
}

fn signed_event_with_content(content: &str) -> nostr::Event {
    EventBuilder::new(Kind::Custom(KIND_TEAM_CATALOG as u16), content)
        .tags(vec![Tag::parse(["d", "team-abc"]).unwrap()])
        .sign_with_keys(&nostr::Keys::generate())
        .unwrap()
}

#[test]
fn test_content_round_trips_through_an_event() {
    let members = [member("m1", "One"), member("m2", "Two")];
    let built = build_team_catalog_content(&team(), &members).unwrap();
    let event = build_team_catalog_event(&team(), &members, true)
        .unwrap()
        .sign_with_keys(&nostr::Keys::generate())
        .unwrap();

    assert_eq!(team_catalog_content_from_event(&event).unwrap(), built);
}

#[test]
fn test_unknown_schema_version_is_rejected() {
    let event = signed_event_with_content(r#"{"v":2,"name":"Future Team","members":[]}"#);

    let error = team_catalog_content_from_event(&event).unwrap_err();

    assert!(
        error.contains("unsupported team catalog schema version 2"),
        "{error}"
    );
}

#[test]
fn test_body_missing_the_version_is_rejected() {
    // `v` has no serde default — body without it cannot masquerade as v1.
    let event = signed_event_with_content(r#"{"name":"No Version","members":[]}"#);

    assert!(team_catalog_content_from_event(&event).is_err());
}

#[test]
fn test_malformed_member_fields_are_rejected() {
    // Wrong-typed field must fail parsing, not silently coerce.
    let event = signed_event_with_content(
        r#"{"v":1,"name":"Bad","members":[{"member_key":"m1","display_name":"One","parallelism":"lots"}]}"#,
    );

    assert!(team_catalog_content_from_event(&event).is_err());
}

#[test]
fn test_inbound_body_over_the_size_contract_is_rejected_on_read() {
    // Readers enforce the same bounds as writers.
    let members: String = (0..=MAX_MEMBERS)
        .map(|i| format!(r#"{{"member_key":"m{i}","display_name":"M{i}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let event = signed_event_with_content(&format!(
        r#"{{"v":1,"name":"Too Many","members":[{members}]}}"#
    ));

    let error = team_catalog_content_from_event(&event).unwrap_err();

    assert!(error.contains("team too large to share"), "{error}");
}

#[test]
fn test_member_key_is_stable_for_an_unchanged_member() {
    let one = member("m1", "One");
    let a = build_team_catalog_content(&team(), std::slice::from_ref(&one)).unwrap();
    let b = build_team_catalog_content(&team(), &[one]).unwrap();

    assert_eq!(a.members[0].member_key, b.members[0].member_key);
    assert!(!a.members[0].member_key.is_empty());
}

#[test]
fn test_member_key_follows_the_member_across_a_reorder() {
    // A position-derived key would re-point every copy after any membership reorder.
    let forward =
        build_team_catalog_content(&team(), &[member("m1", "One"), member("m2", "Two")]).unwrap();
    let reversed =
        build_team_catalog_content(&team(), &[member("m2", "Two"), member("m1", "One")]).unwrap();

    assert_eq!(
        forward.members[0].member_key,
        reversed.members[1].member_key
    );
    assert_eq!(
        forward.members[1].member_key,
        reversed.members[0].member_key
    );
}

#[test]
fn test_two_members_with_identical_content_still_get_distinct_keys() {
    let mut twin = member("m2", "One");
    twin.system_prompt = member("m1", "One").system_prompt.clone();

    let content = build_team_catalog_content(&team(), &[member("m1", "One"), twin]).unwrap();

    assert_ne!(content.members[0].member_key, content.members[1].member_key);
}

#[test]
fn test_ids_that_persona_d_tag_would_collapse_get_distinct_keys() {
    use crate::managed_agents::persona_events::persona_d_tag;

    // Each pair has the same d-tag but must get distinct member keys.
    let long = "x".repeat(64);
    for (left, right) in [
        ("Reviewer".to_string(), "reviewer".to_string()),
        ("a b".to_string(), "a.b".to_string()),
        (format!("{long}1"), format!("{long}2")),
    ] {
        let (one, two) = (member(&left, "One"), member(&right, "Two"));
        assert_eq!(
            persona_d_tag(&one),
            persona_d_tag(&two),
            "fixture must actually collide under the d-tag normalizer"
        );

        let content = build_team_catalog_content(&team(), &[one, two]).unwrap();

        assert_ne!(
            content.members[0].member_key, content.members[1].member_key,
            "'{left}' and '{right}' must not share a published identity"
        );
    }
}

#[test]
fn test_member_key_does_not_disclose_the_local_id() {
    let content = build_team_catalog_content(&team(), &[member("secret-local-id", "One")]).unwrap();

    assert!(!team_catalog_content_json(&content)
        .unwrap()
        .contains("secret-local-id"));
    assert_eq!(
        content.members[0].member_key.len(),
        PROJECTION_HASH_HEX_LEN,
        "a SHA-256 hex digest"
    );
}

#[test]
fn test_a_body_repeating_a_member_key_is_rejected_on_read() {
    // Two members on one key collapse onto a single local persona, silently dropping one.
    let event = signed_event_with_content(
        r#"{"v":1,"name":"Twins","members":[
            {"member_key":"k","display_name":"One"},
            {"member_key":"k","display_name":"Two"}
        ]}"#,
    );

    let error = team_catalog_content_from_event(&event).unwrap_err();

    assert!(error.contains("repeats the member key"), "{error}");
    assert!(
        error.contains("Two"),
        "the error names the offender: {error}"
    );
}

/// A body carrying one member built from `fields`, as JSON.
fn body_with_member(fields: &str) -> nostr::Event {
    signed_event_with_content(&format!(
        r#"{{"v":1,"name":"T","members":[{{"member_key":"k","display_name":"One",{fields}}}]}}"#
    ))
}

#[test]
fn test_members_violating_the_v1_contract_are_rejected_on_read() {
    for (label, fields) in [
        (
            "out-of-range parallelism",
            r#""parallelism":999"#.to_string(),
        ),
        ("zero parallelism", r#""parallelism":0"#.to_string()),
        (
            "unknown respond_to mode",
            r#""respond_to":"everyone""#.to_string(),
        ),
        ("empty runtime", r#""runtime":"""#.to_string()),
        (
            "oversize model",
            format!(r#""model":"{}""#, "m".repeat(MAX_IDENTIFIER_BYTES + 1)),
        ),
        ("empty name-pool entry", r#""name_pool":[""]"#.to_string()),
        (
            "reuse slug with no hash",
            r#""builtin_slug":"reviewer""#.to_string(),
        ),
        (
            "reuse hash with no slug",
            format!(r#""projection_hash":"{}""#, "a".repeat(64)),
        ),
        (
            "malformed reuse hash",
            r#""builtin_slug":"reviewer","projection_hash":"nope""#.to_string(),
        ),
        (
            "non-hex reuse hash",
            format!(
                r#""builtin_slug":"reviewer","projection_hash":"{}""#,
                "z".repeat(64)
            ),
        ),
    ] {
        assert!(
            team_catalog_content_from_event(&body_with_member(&fields)).is_err(),
            "{label} must be refused at the parse boundary"
        );
    }
}

#[test]
fn test_members_at_the_edges_of_the_v1_contract_are_accepted() {
    for (label, fields) in [
        ("minimum parallelism", r#""parallelism":1"#.to_string()),
        ("maximum parallelism", r#""parallelism":32"#.to_string()),
        (
            "well-formed reuse hint",
            format!(
                r#""builtin_slug":"reviewer","projection_hash":"{}""#,
                "A".repeat(64)
            ),
        ),
        (
            "identifier at the limit",
            format!(r#""model":"{}""#, "m".repeat(MAX_IDENTIFIER_BYTES)),
        ),
    ] {
        assert!(
            team_catalog_content_from_event(&body_with_member(&fields)).is_ok(),
            "{label} is within the contract and must be accepted"
        );
    }
}

#[test]
fn test_a_member_with_an_empty_key_or_name_is_rejected_on_read() {
    for members in [
        r#"{"member_key":"","display_name":"One"}"#,
        r#"{"member_key":"k","display_name":"  "}"#,
    ] {
        let event =
            signed_event_with_content(&format!(r#"{{"v":1,"name":"T","members":[{members}]}}"#));
        assert!(
            team_catalog_content_from_event(&event).is_err(),
            "{members}"
        );
    }
}

#[test]
fn test_an_oversize_member_key_is_rejected_on_read() {
    let event = signed_event_with_content(&format!(
        r#"{{"v":1,"name":"T","members":[{{"member_key":"{}","display_name":"One"}}]}}"#,
        "k".repeat(MAX_MEMBER_KEY_BYTES + 1)
    ));

    assert!(team_catalog_content_from_event(&event).is_err());
}

#[test]
fn test_catalog_delete_targets_the_30178_coordinate_with_no_e_tag() {
    const OWNER: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    let event = build_team_catalog_delete("team-abc", OWNER)
        .unwrap()
        .sign_with_keys(&nostr::Keys::generate())
        .unwrap();

    assert_eq!(event.kind, Kind::Custom(5));
    let a_tags: Vec<&[String]> = event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter(|parts| parts.first().map(String::as_str) == Some("a"))
        .collect();
    assert_eq!(a_tags.len(), 1);
    assert_eq!(
        a_tags[0][1],
        format!("{KIND_TEAM_CATALOG}:{OWNER}:team-abc")
    );
    // An e-tag would leave the replaceable coordinate live.
    assert!(event
        .tags
        .iter()
        .all(|tag| tag.as_slice().first().map(String::as_str) != Some("e")));
}

macro_rules! fixture {
    ($name:literal) => {
        include_str!(concat!(
            "../../../tests/fixtures/team_catalog_content/",
            $name
        ))
    };
}

/// Run the parser on each named fixture; `$expect_ok` determines pass/fail.
macro_rules! run_fixture_table {
    ($fn_name:ident, $expect_ok:expr, $( ($name:literal, $file:literal $(, $note:literal)?) ),+ $(,)?) => {
        #[test]
        fn $fn_name() {
            for (name, body) in [$( ($name, fixture!($file)) ),+] {
                let event = signed_event_with_content(body.trim());
                if $expect_ok {
                    assert!(
                        team_catalog_content_from_event(&event).is_ok(),
                        "{name}.json must be accepted"
                    );
                } else {
                    assert!(
                        team_catalog_content_from_event(&event).is_err(),
                        "{name}.json must be rejected"
                    );
                }
            }
        }
    };
}

run_fixture_table!(
    test_fixtures_that_must_be_accepted_are_accepted,
    true,
    ("valid_minimal", "valid_minimal.json"),
    (
        "valid_respond_to_owner_only",
        "valid_respond_to_owner_only.json"
    ),
    (
        "valid_respond_to_allowlist",
        "valid_respond_to_allowlist.json"
    ),
    ("valid_respond_to_anyone", "valid_respond_to_anyone.json"),
    ("valid_uppercase_hash", "valid_uppercase_hash.json"),
    ("valid_avatar_url_https", "valid_avatar_url_https.json"),
    (
        "valid_avatar_url_uppercase_scheme",
        "valid_avatar_url_uppercase_scheme.json"
    ),
    (
        "valid_avatar_url_non_ascii_at_utf8_limit",
        "valid_avatar_url_non_ascii_at_utf8_limit.json"
    ),
    (
        "valid_avatar_url_shorthand_scheme",
        "valid_avatar_url_shorthand_scheme.json"
    ),
    (
        "valid_avatar_url_unicode_nel",
        "valid_avatar_url_unicode_nel.json"
    ),
);

run_fixture_table!(
    test_fixtures_that_must_be_rejected_are_rejected,
    false,
    (
        "invalid_respond_to_pascal_case",
        "invalid_respond_to_pascal_case.json"
    ),
    (
        "invalid_description_wrong_type",
        "invalid_description_wrong_type.json"
    ),
    (
        "invalid_instructions_wrong_type",
        "invalid_instructions_wrong_type.json"
    ),
    (
        "invalid_duplicate_member_key",
        "invalid_duplicate_member_key.json"
    ),
    (
        "invalid_name_pool_not_array",
        "invalid_name_pool_not_array.json"
    ),
    ("invalid_name_pool_null", "invalid_name_pool_null.json"),
    (
        "invalid_builtin_slug_wrong_type",
        "invalid_builtin_slug_wrong_type.json"
    ),
    (
        "invalid_avatar_url_javascript",
        "invalid_avatar_url_javascript.json"
    ),
    ("invalid_team_name_blank", "invalid_team_name_blank.json"),
    (
        "invalid_avatar_url_bare_https",
        "invalid_avatar_url_bare_https.json"
    ),
    (
        "invalid_avatar_url_whitespace_in_url",
        "invalid_avatar_url_whitespace_in_url.json"
    ),
    (
        "invalid_avatar_url_https_over_2048",
        "invalid_avatar_url_https_over_2048.json"
    ),
    (
        "invalid_avatar_url_malformed_port",
        "invalid_avatar_url_malformed_port.json"
    ),
    (
        "invalid_avatar_url_non_ascii_over_utf8_limit",
        "invalid_avatar_url_non_ascii_over_utf8_limit.json"
    ),
    (
        "invalid_avatar_url_unicode_nbsp",
        "invalid_avatar_url_unicode_nbsp.json"
    ),
    (
        "invalid_avatar_url_unicode_em_space",
        "invalid_avatar_url_unicode_em_space.json"
    ),
    (
        "invalid_avatar_url_unicode_bom",
        "invalid_avatar_url_unicode_bom.json"
    ),
);

#[test]
fn test_real_builtin_without_avatar_mutation_projects_successfully() {
    // A real built-in (fizz) has a ~170 KiB oversized avatar that is stripped in member_projection.
    let builtin =
        crate::managed_agents::built_in_persona_definition("builtin:fizz", "2026-07-30T00:00:00Z")
            .expect("builtin:fizz must exist");
    let has_large_avatar = builtin
        .avatar_url
        .as_deref()
        .is_some_and(|url| url.len() > MAX_AVATAR_URL_BYTES);
    let mut t = team();
    t.instructions = None;
    let content = build_team_catalog_content(&t, &[builtin]).expect(
        "a team containing a real built-in must project successfully without avatar mutation",
    );
    assert_eq!(content.members.len(), 1);
    if has_large_avatar {
        assert!(
            content.members[0].avatar_url.is_none(),
            "oversized built-in avatar must be omitted, not rejected"
        );
    }
    assert!(
        validate_team_catalog_content(&content).is_ok(),
        "projected content must pass full validation"
    );
}

#[test]
fn test_tombstone_transaction_rolls_back_delete_when_insert_fails() {
    // Use a BEFORE INSERT trigger to force the INSERT step to fail; verify DELETE is rolled back.
    use crate::managed_agents::retention::{
        get_retained_event, open_retention_db, retain_event, scoped_retention_db_path,
        RetainedEvent,
    };
    use buzz_core_pkg::kind::KIND_TEAM_CATALOG;
    use nostr::JsonUtil;

    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_retention_db_path(dir.path(), "wss://a.example", &owner);
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

    let t = team();
    let m = member("m1", "Sentinel.");
    let head_event = build_team_catalog_event(&t, &[m], true)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    let conn = open_retention_db(&db_path).unwrap();
    retain_event(
        &conn,
        &RetainedEvent {
            kind: KIND_TEAM_CATALOG,
            pubkey: owner.clone(),
            d_tag: "team-abc".to_string(),
            content: head_event.content.to_string(),
            created_at: head_event.created_at.as_secs() as i64,
            raw_event: head_event.as_json(),
            pending_sync: false,
        },
    )
    .unwrap();

    conn.execute_batch(
        "CREATE TRIGGER block_all_inserts BEFORE INSERT ON persona_events
         BEGIN
             SELECT RAISE(ABORT, 'insert blocked by test trigger');
         END;",
    )
    .unwrap();
    drop(conn);

    let result = tombstone_team_catalog_coordinate(&db_path, &keys, "team-abc");
    assert!(result.is_err(), "tombstone with INSERT trigger must fail");
    let err = result.unwrap_err();
    let blocked = err.contains("insert blocked by test trigger") || err.contains("blocked");
    assert!(blocked, "error must name the trigger cause; got: {err}");

    let conn = open_retention_db(&db_path).unwrap();
    let head = get_retained_event(&conn, KIND_TEAM_CATALOG, &owner, "team-abc").unwrap();
    assert!(head.is_some());
}

#[test]
fn test_oversized_inline_raster_avatar_on_non_builtin_is_downscaled() {
    // 300×300 gradient PNG data URL exceeds MAX_AVATAR_URL_BYTES.
    let img = image::RgbaImage::from_fn(300, 300, |x, y| {
        image::Rgba([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8, 255])
    });
    let mut raw = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut raw);
    img.write_to(&mut cursor, image::ImageFormat::Png).unwrap();
    let url = format!("data:image/png;base64,{}", STANDARD.encode(&raw));
    assert!(url.len() > MAX_AVATAR_URL_BYTES);
    let mut one = member("m1", "Avatar Hog");
    one.avatar_url = Some(url);
    let content = build_team_catalog_content(&team(), &[one]).unwrap();
    let pav = content.members[0].avatar_url.as_deref().unwrap();
    assert!(pav.len() <= MAX_AVATAR_URL_BYTES && is_safe_catalog_avatar_url(pav));
}

#[test]
fn test_undecodable_oversized_data_url_falls_through_to_validation_error() {
    let cap = MAX_AVATAR_URL_BYTES;
    let url = format!("data:image/png;base64,{}", "!!!".repeat(cap / 3 + 1));
    let mut one = member("m1", "Bad Avatar");
    one.avatar_url = Some(url);
    let error = build_team_catalog_content(&team(), &[one]).unwrap_err();
    assert!(error.contains("avatar") || error.contains("too large"));
}

#[test]
fn test_extreme_dimension_avatar_falls_through_to_validation_error() {
    // 2100×2100 PNG exceeds the 2048px decode ceiling; bounded decoder rejects it before pixel allocation.
    let img = image::RgbaImage::from_fn(2100, 2100, |x, y| {
        image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
    });
    let mut raw = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut raw), image::ImageFormat::Png)
        .unwrap();
    let url = format!("data:image/png;base64,{}", STANDARD.encode(&raw));
    assert!(
        url.len() > MAX_AVATAR_URL_BYTES,
        "fixture must be oversized"
    );
    let mut one = member("m1", "Bomb");
    one.avatar_url = Some(url);
    let error = build_team_catalog_content(&team(), &[one]).unwrap_err();
    assert!(
        error.contains("avatar") || error.contains("too large"),
        "{error}"
    );
}
