//! Behavior tests for `add_team_from_catalog`: A2 (backend head acceptance) and
//! A1 (local store planning). No Tauri app or relay needed.

use super::{apply::plan_add, normalized_event_id, verified_head_content};
use crate::managed_agents::{
    team_catalog::{
        build_team_catalog_event, local_member_projection_hash, TeamCatalogContent,
        TeamCatalogMember, MAX_MEMBERS, TEAM_CATALOG_SCHEMA_VERSION,
    },
    AgentDefinition, TeamCatalogSource, TeamRecord,
};
use nostr::{EventBuilder, JsonUtil, Kind, Tag};
use std::collections::BTreeMap;
mod concealment; // executable-text concealment gate (Carl P1)
mod retention; // adoption-path retention enqueue (Wes/Carl P1)
mod reuse; // built-in reuse decision (`reusable_builtin`)
mod scope_fence; // adoption community-boundary fence (Carl r11 P1)

const NOW: &str = "2026-07-30T00:00:00Z";
const TEAM_D_TAG: &str = "team-alpha";

fn persona(id: &str, prompt: &str) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        display_name: id.to_string(),
        description: None,
        avatar_url: None,
        system_prompt: prompt.to_string(),
        runtime: None,
        model: None,
        provider: None,
        name_pool: Vec::new(),
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
        created_at: NOW.to_string(),
        updated_at: NOW.to_string(),
    }
}

fn member(member_key: &str, prompt: &str) -> TeamCatalogMember {
    TeamCatalogMember {
        member_key: member_key.to_string(),
        display_name: member_key.to_string(),
        system_prompt: Some(prompt.to_string()),
        avatar_url: None,
        runtime: None,
        model: None,
        provider: None,
        name_pool: Vec::new(),
        respond_to: None,
        parallelism: None,
        builtin_slug: None,
        projection_hash: None,
    }
}

fn content(members: Vec<TeamCatalogMember>) -> TeamCatalogContent {
    TeamCatalogContent {
        v: TEAM_CATALOG_SCHEMA_VERSION,
        name: "Alpha".to_string(),
        description: Some("The alpha team.".to_string()),
        instructions: None,
        members,
    }
}

fn source(owner_pubkey: &str) -> TeamCatalogSource {
    TeamCatalogSource {
        owner_pubkey: owner_pubkey.to_string(),
        team_d_tag: TEAM_D_TAG.to_string(),
    }
}

/// A signed 30178 head for `team` + `members`, plus its owner and source.
fn published(
    team: &TeamRecord,
    members: &[AgentDefinition],
    shared: bool,
) -> (nostr::Event, TeamCatalogSource) {
    let keys = nostr::Keys::generate();
    let event = build_team_catalog_event(team, members, shared)
        .expect("the fixture team is within the size contract")
        .sign_with_keys(&keys)
        .expect("signing a locally built event cannot fail");
    let source = source(&keys.public_key().to_hex());
    (event, source)
}

fn team_fixture(persona_ids: Vec<String>) -> TeamRecord {
    TeamRecord {
        id: TEAM_D_TAG.to_string(),
        name: "Alpha".to_string(),
        description: Some("The alpha team.".to_string()),
        instructions: None,
        persona_ids,
        is_builtin: false,
        shared: false,
        catalog_source: None,
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: NOW.to_string(),
        updated_at: NOW.to_string(),
    }
}

// ── Event-id normalization ───────────────────────────────────────────────────

#[test]
fn test_uppercase_event_id_normalizes_to_lowercase() {
    // Head ids compared as strings against `Event::id().to_hex()` (always lowercase).
    let normalized = normalized_event_id(&format!("  {}  ", "A".repeat(64)))
        .expect("64 hex chars with surrounding space is valid");
    assert_eq!(normalized, "a".repeat(64));
}

#[test]
fn test_short_event_id_is_rejected() {
    let error = normalized_event_id("abc123").unwrap_err();
    assert!(
        error.contains("64 hex"),
        "error must name the rule: {error}"
    );
}

#[test]
fn test_non_hex_event_id_is_rejected() {
    let error = normalized_event_id(&"z".repeat(64)).unwrap_err();
    assert!(
        error.contains("64 hex"),
        "error must name the rule: {error}"
    );
}

// ── Head verification (A2) ───────────────────────────────────────────────────

#[test]
fn test_matching_shared_head_yields_its_projection() {
    let (event, source) = published(
        &team_fixture(vec!["m1".to_string()]),
        &[persona("m1", "Do the work.")],
        true,
    );

    let parsed = verified_head_content(&event, &source, &event.id.to_hex())
        .expect("a signed, shared head at the requested coordinate is acceptable");

    assert_eq!(parsed.name, "Alpha");
    assert_eq!(parsed.members.len(), 1);
}

#[test]
fn test_head_that_moved_since_the_dialog_opened_is_rejected() {
    // Owner republished between catalog render and click — stale head must fail.
    let (event, source) = published(
        &team_fixture(vec!["m1".to_string()]),
        &[persona("m1", "Do the work.")],
        true,
    );

    let error = verified_head_content(&event, &source, &"a".repeat(64)).unwrap_err();

    assert!(
        error.contains("changed"),
        "the rejection must tell the user to refresh: {error}"
    );
}

#[test]
fn test_unshared_head_is_rejected() {
    // Unshare replaces the head with an untagged event; stale readers must not be able to add it.
    let (event, source) = published(
        &team_fixture(vec!["m1".to_string()]),
        &[persona("m1", "Do the work.")],
        false,
    );

    let error = verified_head_content(&event, &source, &event.id.to_hex()).unwrap_err();

    assert!(
        error.contains("no longer shared"),
        "the rejection must name the withdrawal: {error}"
    );
}

#[test]
fn test_head_from_a_different_owner_is_rejected() {
    // Hostile relay answering an `authors` filter with another publisher's event must fail.
    let (event, _) = published(
        &team_fixture(vec!["m1".to_string()]),
        &[persona("m1", "Do the work.")],
        true,
    );

    let error =
        verified_head_content(&event, &source(&"a".repeat(64)), &event.id.to_hex()).unwrap_err();

    assert!(
        error.contains("different owner"),
        "the rejection must name the mismatch: {error}"
    );
}

#[test]
fn test_head_for_a_different_team_is_rejected() {
    let (event, source) = published(
        &team_fixture(vec!["m1".to_string()]),
        &[persona("m1", "Do the work.")],
        true,
    );
    let other_team = TeamCatalogSource {
        team_d_tag: "team-beta".to_string(),
        ..source
    };

    let error = verified_head_content(&event, &other_team, &event.id.to_hex()).unwrap_err();

    assert!(
        error.contains("different team"),
        "the rejection must name the mismatch: {error}"
    );
}

#[test]
fn test_head_of_the_wrong_kind_is_rejected() {
    // 30176 is the owner's private wire shape, not a catalog projection.
    let keys = nostr::Keys::generate();
    let event = EventBuilder::new(Kind::Custom(30176), "{}")
        .tags(vec![Tag::parse(["d", TEAM_D_TAG]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();

    let error = verified_head_content(
        &event,
        &source(&keys.public_key().to_hex()),
        &event.id.to_hex(),
    )
    .unwrap_err();

    assert!(
        error.contains("not a team publication"),
        "the rejection must name the kind mismatch: {error}"
    );
}

#[test]
fn test_head_with_a_forged_signature_is_rejected() {
    // Without this check, a hostile relay could set both `pubkey` and `content`.
    let (event, source) = published(
        &team_fixture(vec!["m1".to_string()]),
        &[persona("m1", "Do the work.")],
        true,
    );
    let mut json: serde_json::Value = serde_json::from_str(&event.as_json()).unwrap();
    json["content"] = serde_json::json!(r#"{"v":1,"name":"Trojan","members":[]}"#);
    let tampered = <nostr::Event as nostr::JsonUtil>::from_json(json.to_string()).unwrap();

    let error = verified_head_content(&tampered, &source, &tampered.id.to_hex()).unwrap_err();

    assert!(
        error.contains("signature"),
        "content edits must fail signature verification: {error}"
    );
}

#[test]
fn test_head_with_two_d_tags_is_rejected() {
    // Relay's A4 gate rejects these; a reader taking the first d-tag would resolve an unverified coordinate.
    let keys = nostr::Keys::generate();
    let body = serde_json::to_string(&content(vec![member("m1", "Do the work.")])).unwrap();
    let event = EventBuilder::new(Kind::Custom(30178), body)
        .tags(vec![
            Tag::parse(["d", TEAM_D_TAG]).unwrap(),
            Tag::parse(["d", "team-beta"]).unwrap(),
            Tag::parse(["shared", "true"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();

    let error = verified_head_content(
        &event,
        &source(&keys.public_key().to_hex()),
        &event.id.to_hex(),
    )
    .unwrap_err();

    assert!(
        error.contains("different team"),
        "an ambiguous d-tag resolves to no coordinate: {error}"
    );
}

#[test]
fn test_head_with_an_unknown_schema_version_is_rejected() {
    let keys = nostr::Keys::generate();
    let event = EventBuilder::new(
        Kind::Custom(30178),
        r#"{"v":2,"name":"Alpha","members":[]}"#,
    )
    .tags(vec![
        Tag::parse(["d", TEAM_D_TAG]).unwrap(),
        Tag::parse(["shared", "true"]).unwrap(),
    ])
    .sign_with_keys(&keys)
    .unwrap();

    let error = verified_head_content(
        &event,
        &source(&keys.public_key().to_hex()),
        &event.id.to_hex(),
    )
    .unwrap_err();

    assert!(
        error.contains("schema version"),
        "a v2 body may reshape any field: {error}"
    );
}

#[test]
fn test_head_that_violates_the_size_contract_is_rejected() {
    // Publisher bypassing the local builder must not force an unbounded projection.
    let keys = nostr::Keys::generate();
    let members = (0..=MAX_MEMBERS)
        .map(|i| member(&format!("m{i}"), "Do the work."))
        .collect();
    let body = serde_json::to_string(&content(members)).unwrap();
    let event = EventBuilder::new(Kind::Custom(30178), body)
        .tags(vec![
            Tag::parse(["d", TEAM_D_TAG]).unwrap(),
            Tag::parse(["shared", "true"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();

    let error = verified_head_content(
        &event,
        &source(&keys.public_key().to_hex()),
        &event.id.to_hex(),
    )
    .unwrap_err();

    assert!(
        error.contains("too large"),
        "the size contract applies on read as well as write: {error}"
    );
}

// ── Store planning (A1 provenance) ───────────────────────────────────────────

fn plan(
    personas: &[AgentDefinition],
    teams: &[TeamRecord],
    source: &TeamCatalogSource,
    content: &TeamCatalogContent,
) -> super::apply::AddPlan {
    plan_add(personas, teams, source, content, NOW).expect("the fixture projection is resolvable")
}

#[test]
fn test_first_add_copies_every_member_and_records_provenance() {
    let source = source(&"a".repeat(64));
    let body = content(vec![member("m1", "Do the work."), member("m2", "Review.")]);

    let plan = plan(&[], &[], &source, &body);

    let (personas, teams) = plan.stores.expect("a first add must write");
    assert_eq!(personas.len(), 2);
    assert_eq!(teams.len(), 1);
    assert_eq!(
        plan.team.catalog_source.as_ref(),
        Some(&source),
        "the copy's only link back to the publication"
    );
    assert!(
        !plan.team.shared,
        "a copy is not published; sharing it is a separate act by its new owner"
    );
    assert_eq!(
        plan.team.persona_ids,
        personas.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
        "membership must preserve the published order"
    );
    for copy in &personas {
        let held = copy
            .team_catalog_source
            .as_ref()
            .expect("every copy carries team provenance");
        assert_eq!(held.owner_pubkey, source.owner_pubkey);
        assert_eq!(held.team_d_tag, source.team_d_tag);
        assert!(
            copy.catalog_source.is_none(),
            "a team member is not addressable as a 30175 persona coordinate"
        );
    }
}

#[test]
fn test_adding_the_same_publication_twice_writes_nothing() {
    let source = source(&"a".repeat(64));
    let body = content(vec![member("m1", "Do the work.")]);
    let first = plan(&[], &[], &source, &body);
    let (personas, teams) = first.stores.unwrap();

    let second = plan(&personas, &teams, &source, &body);

    assert!(
        second.stores.is_none(),
        "a replay must not mint a second copy"
    );
    assert_eq!(second.team.id, first.team.id);
}

#[test]
fn test_a_second_team_by_the_same_publisher_gets_its_own_member_copies() {
    // Reuse scoped to one publication: sharing a copy across teams would let deleting either orphan it.
    let source = source(&"a".repeat(64));
    let body = content(vec![member("m1", "Do the work.")]);
    let (personas, teams) = plan(&[], &[], &source, &body).stores.unwrap();
    let other_publication = TeamCatalogSource {
        team_d_tag: "team-beta".to_string(),
        ..source
    };

    let (after, _) = plan(&personas, &teams, &other_publication, &body)
        .stores
        .expect("a different team d-tag is a new add");

    assert_eq!(
        after.len(),
        2,
        "an identical member from a different publication is its own copy"
    );
}

#[test]
fn test_a_deactivated_copy_is_reactivated_rather_than_duplicated() {
    // `delete_team_with_cascade` deactivates copies; re-adding must revive them, not stack a second set.
    // (verifies `plan_add`'s reactivation branch; production deactivation path in `teams_tests`).
    let source = source(&"a".repeat(64));
    let body = content(vec![member("m1", "Do the work.")]);
    let (mut personas, _) = plan(&[], &[], &source, &body).stores.unwrap();
    personas[0].is_active = false; // mirrors what delete_team_with_cascade does

    let (after, _) = plan(&personas, &[], &source, &body)
        .stores
        .expect("with the team gone, this is a fresh add");

    assert_eq!(after.len(), 1, "the existing copy is reused");
    assert!(after[0].is_active, "and reactivated");
}

#[test]
fn test_a_newer_version_of_a_member_becomes_a_separate_copy() {
    // Provenance match is on triple (owner, d_tag, member_key, prompt): adding newer version is a distinct copy.
    let source = source(&"a".repeat(64));
    let (personas, _) = plan(&[], &[], &source, &content(vec![member("m1", "Old.")]))
        .stores
        .unwrap();
    let (after, _) = plan(
        &personas,
        &[],
        &source,
        &content(vec![member("m1", "New.")]),
    )
    .stores
    .unwrap();
    assert_eq!(after.len(), 2, "a changed member is a distinct version");
    assert_ne!(
        after[0]
            .team_catalog_source
            .as_ref()
            .map(|s| &s.projection_hash),
        after[1]
            .team_catalog_source
            .as_ref()
            .map(|s| &s.projection_hash),
    );
}

#[test]
fn test_a_copy_inherits_no_secrets_and_no_audience() {
    let source = source(&"a".repeat(64));
    let mut published = member("m1", "Do the work.");
    published.respond_to = Some("anyone".to_string());

    let (after, _) = plan(&[], &[], &source, &content(vec![published]))
        .stores
        .unwrap();

    let copy = &after[0];
    assert!(copy.env_vars.is_empty(), "env vars are never projected");
    assert!(
        copy.respond_to_allowlist.is_empty(),
        "an allowlist is the owner's social graph and is never inherited"
    );
    assert_eq!(copy.respond_to.as_deref(), Some("anyone"));
    assert!(!copy.shared, "a copy is not itself published");
}

#[test]
fn test_an_unrecognized_respond_to_mode_fails_the_whole_add() {
    // Copying an unknown mode opaquely would give the copy an audience the
    // recipient's UI cannot render — and cannot be trusted to be restrictive.
    let source = source(&"a".repeat(64));
    let mut published = member("m1", "Do the work.");
    published.respond_to = Some("everyone-forever".to_string());

    let error = plan_add(&[], &[], &source, &content(vec![published]), NOW).unwrap_err();

    assert!(
        error.contains("not a recognized mode"),
        "the failure must name the bad mode: {error}"
    );
}

#[test]
fn test_a_failed_member_leaves_the_plan_unwritten() {
    // All-or-nothing before any I/O: a failed member leaves no earlier members written.
    let source = source(&"a".repeat(64));
    let mut bad = member("m2", "Do the work.");
    bad.respond_to = Some("everyone-forever".to_string());

    let resolved = plan_add(
        &[],
        &[],
        &source,
        &content(vec![member("m1", "Do the work."), bad]),
        NOW,
    );

    assert!(
        resolved.is_err(),
        "no partial plan is returned when a member cannot be resolved"
    );
}

#[test]
fn test_an_empty_publication_adds_a_team_with_no_members() {
    // A team whose every member was deleted still projects; adding it must
    // produce an empty team rather than failing or inventing a member.
    let source = source(&"a".repeat(64));

    let plan = plan(&[], &[], &source, &content(Vec::new()));

    let (personas, teams) = plan.stores.expect("an empty team is still an add");
    assert!(personas.is_empty());
    assert_eq!(teams.len(), 1);
    assert!(plan.team.persona_ids.is_empty());
}

#[test]
fn test_provenance_from_a_different_owner_does_not_match() {
    // Two publishers can legitimately use the same team d-tag and member key.
    let mine = source(&"a".repeat(64));
    let theirs = source(&"b".repeat(64));
    let body = content(vec![member("m1", "Do the work.")]);
    let (personas, _) = plan(&[], &[], &mine, &body).stores.unwrap();

    let (after, _) = plan(&personas, &[], &theirs, &body).stores.unwrap();

    assert_eq!(
        after.len(),
        2,
        "provenance is scoped to the publishing owner"
    );
}

#[test]
fn test_a_persona_catalog_copy_is_not_mistaken_for_a_team_member() {
    // 30175 and 30178 are different namespaces; a persona-catalog copy must not satisfy team provenance.
    let source = source(&"a".repeat(64));
    let mut persona_copy = persona("p1", "Do the work.");
    persona_copy.catalog_source = Some(crate::managed_agents::CatalogSource {
        owner_pubkey: source.owner_pubkey.clone(),
        persona_id: "m1".to_string(),
    });

    let (after, _) = plan(
        &[persona_copy],
        &[],
        &source,
        &content(vec![member("m1", "Do the work.")]),
    )
    .stores
    .unwrap();

    assert_eq!(after.len(), 2, "the 30175 copy is not a 30178 member");
}

#[test]
fn test_provenance_survives_a_store_round_trip() {
    // Reuse reads from disk; a provenance field that does not persist would silently duplicate copies.
    let src = source(&"a".repeat(64));
    let (personas, _) = plan(&[], &[], &src, &content(vec![member("m1", "Do it.")]))
        .stores
        .unwrap();
    let json = serde_json::to_string(&personas).unwrap();
    let reloaded: Vec<AgentDefinition> = serde_json::from_str(&json).unwrap();
    assert_eq!(
        reloaded[0].team_catalog_source.clone(),
        personas[0].team_catalog_source.clone(),
    );
}

// ── Lifecycle: delete seam + re-add, allowlist normalization, built-in round-trip

#[test]
fn test_delete_catalog_team_seam_then_re_add_reactivates_copies() {
    // Exercises delete_catalog_team_at (the production file-based seam) + re-add.
    let dir = tempfile::tempdir().unwrap();
    let src = TeamCatalogSource {
        owner_pubkey: "f".repeat(64),
        team_d_tag: "team-delta".to_string(),
    };
    let body = content(vec![member("mk1", "Do it.")]);
    let (personas, teams) = plan_add(&[], &[], &src, &body, NOW)
        .unwrap()
        .stores
        .unwrap();
    let copy_id = personas[0].id.clone();
    let (pp, tp) = (dir.path().join("p.json"), dir.path().join("t.json"));
    std::fs::write(&pp, serde_json::to_string(&personas).unwrap()).unwrap();
    std::fs::write(&tp, serde_json::to_string(&teams).unwrap()).unwrap();
    crate::managed_agents::delete_catalog_team_at(&pp, &tp, &teams[0].id).unwrap();
    let del_p: Vec<AgentDefinition> =
        serde_json::from_str(&std::fs::read_to_string(&pp).unwrap()).unwrap();
    let del_t: Vec<TeamRecord> =
        serde_json::from_str(&std::fs::read_to_string(&tp).unwrap()).unwrap();
    assert!(
        del_t.is_empty() && !del_p[0].is_active,
        "delete must remove team and deactivate copy"
    );
    let (after, _) = plan_add(&del_p, &del_t, &src, &body, NOW)
        .unwrap()
        .stores
        .unwrap();
    assert_eq!(after[0].id, copy_id, "re-add reuses same copy id");
    assert!(after[0].is_active, "copy is reactivated");
}

#[test]
fn test_allowlist_respond_to_is_normalized_to_owner_only_on_adoption() {
    // The publisher's allowlist is their social graph and must not be copied.
    // The mode itself downgrades to owner-only so the copy is launch-valid.
    let src = source(&"e".repeat(64));
    let mut m = member("m1", "Review the work.");
    m.respond_to = Some("allowlist".to_string());
    let (personas, _) = plan(&[], &[], &src, &content(vec![m])).stores.unwrap();
    assert_eq!(
        personas[0].respond_to.as_deref(),
        Some("owner-only"),
        "allowlist mode must be normalized to owner-only at adoption"
    );
    assert!(personas[0].respond_to_allowlist.is_empty());
    let mint = crate::managed_agents::resolve_mint_behavioral_defaults(
        personas[0]
            .respond_to
            .as_deref()
            .and_then(|w| crate::managed_agents::RespondTo::parse_wire(w).ok()),
        personas[0].respond_to_allowlist.clone(),
        None,
        None,
    );
    assert!(
        mint.is_ok(),
        "normalized respond_to must be launch-valid: {mint:?}"
    );
}

#[test]
fn test_real_builtin_round_trips_through_publish_and_plan_add() {
    // End-to-end reuse fix: fizz (with its ~170 KiB avatar) is published via
    // build_team_catalog_event, parsed on the recipient side, and plan_add
    // reuses the local built-in rather than minting a copy.
    use crate::managed_agents::team_catalog::{
        build_team_catalog_event, team_catalog_content_from_event, MAX_AVATAR_URL_BYTES,
    };
    let local = crate::managed_agents::built_in_persona_definition("builtin:fizz", NOW)
        .expect("builtin:fizz must exist");
    let t = team_fixture(vec![local.id.clone()]);
    let keys = nostr::Keys::generate();
    let event = build_team_catalog_event(&t, std::slice::from_ref(&local), true)
        .expect("real built-in projects without avatar mutation")
        .sign_with_keys(&keys)
        .unwrap();
    let src = source(&keys.public_key().to_hex());
    let body = team_catalog_content_from_event(&event).expect("projected event must parse");
    if local
        .avatar_url
        .as_deref()
        .is_some_and(|u| u.len() > MAX_AVATAR_URL_BYTES)
    {
        assert!(
            body.members[0].avatar_url.is_none(),
            "oversized avatar stripped"
        );
    }
    let (after, _) = plan_add(std::slice::from_ref(&local), &[], &src, &body, NOW)
        .expect("add with matching built-in must succeed")
        .stores
        .expect("add must produce stores");
    assert_eq!(
        after[0].id, local.id,
        "local built-in is reused, no copy minted"
    );
}

// ── commit_stores: byte-level rollback coverage ───────────────────────────

mod commit_stores_tests {
    use super::super::apply::commit_stores;
    use std::fs;

    fn write_file(path: &std::path::Path, contents: &[u8]) {
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn test_both_writes_succeed_leaves_new_content() {
        let dir = tempfile::tempdir().unwrap();
        let personas = dir.path().join("personas.json");
        let teams = dir.path().join("teams.json");
        write_file(&personas, b"old-personas");
        write_file(&teams, b"old-teams");

        let result = commit_stores(
            &personas,
            &teams,
            || {
                fs::write(&personas, b"new-personas").map_err(|e| e.to_string())?;
                Ok(())
            },
            || {
                fs::write(&teams, b"new-teams").map_err(|e| e.to_string())?;
                Ok(())
            },
        );

        assert!(result.is_ok());
        assert_eq!(fs::read(&personas).unwrap(), b"new-personas");
        assert_eq!(fs::read(&teams).unwrap(), b"new-teams");
    }

    #[test]
    fn test_first_write_fails_both_files_restored() {
        let dir = tempfile::tempdir().unwrap();
        let personas = dir.path().join("personas.json");
        let teams = dir.path().join("teams.json");
        write_file(&personas, b"original-personas");
        write_file(&teams, b"original-teams");

        let result = commit_stores(
            &personas,
            &teams,
            || Err("personas save failed".to_string()),
            || unreachable!("teams write should not run if personas failed"),
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("personas save failed"));
        assert_eq!(fs::read(&personas).unwrap(), b"original-personas");
        assert_eq!(fs::read(&teams).unwrap(), b"original-teams");
    }

    #[test]
    fn test_second_write_fails_after_first_committed_both_restored() {
        let dir = tempfile::tempdir().unwrap();
        let personas = dir.path().join("personas.json");
        let teams = dir.path().join("teams.json");
        write_file(&personas, b"original-personas");
        write_file(&teams, b"original-teams");

        let result = commit_stores(
            &personas,
            &teams,
            || {
                fs::write(&personas, b"new-personas").map_err(|e| e.to_string())?;
                Ok(())
            },
            || Err("teams save failed".to_string()),
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("teams save failed"));
        assert_eq!(fs::read(&personas).unwrap(), b"original-personas");
        assert_eq!(fs::read(&teams).unwrap(), b"original-teams");
    }

    #[test]
    fn test_absent_file_is_removed_on_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let personas = dir.path().join("personas.json");
        let teams = dir.path().join("teams.json");

        let result = commit_stores(
            &personas,
            &teams,
            || {
                fs::write(&personas, b"new-personas").map_err(|e| e.to_string())?;
                Ok(())
            },
            || Err("teams save failed".to_string()),
        );

        assert!(result.is_err());
        assert!(
            !personas.exists(),
            "newly created file should be removed on rollback"
        );
        assert!(!teams.exists());
    }

    #[test]
    fn test_restore_failure_message_includes_both_errors() {
        // Restore failure aggregates both original error and restore error.
        // Trigger restore failure by removing the parent dir after snapshotting.
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let personas = sub.join("personas.json");
        let teams = sub.join("teams.json");
        write_file(&personas, b"snap-p");
        write_file(&teams, b"snap-t");

        let sub_clone = sub.clone();
        let result = commit_stores(
            &personas,
            &teams,
            || {
                let _ = std::fs::remove_dir_all(&sub_clone);
                Err("original error".to_string())
            },
            || unreachable!(),
        );

        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("original error"),
            "missing original error in: {msg}"
        );
        assert!(
            msg.contains("could not be restored"),
            "missing restore-failure note in: {msg}"
        );
    }

    #[test]
    fn test_second_position_restore_failure_reported() {
        // Second restore (teams) failure must be reported alongside original error.
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let personas = sub.join("personas.json");
        let teams = sub.join("teams.json");
        write_file(&personas, b"snap-p");
        write_file(&teams, b"snap-t");

        let sub_clone = sub.clone();
        let result = commit_stores(
            &personas,
            &teams,
            || {
                fs::write(&personas, b"new-personas").map_err(|e| e.to_string())?;
                Ok(())
            },
            || {
                let _ = std::fs::remove_dir_all(&sub_clone);
                Err("teams save failed".to_string())
            },
        );

        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("teams save failed"),
            "original teams error missing in: {msg}"
        );
        assert!(
            msg.contains("could not be restored"),
            "restore-failure note missing in: {msg}"
        );
    }

    #[test]
    fn test_absent_snap_restore_is_noop_and_both_restores_are_independent() {
        // Part A — absent snap: when no file existed before the add and the
        // write fails, removing a non-existent path is treated as success
        // (desired state already reached, I5). No "could not be restored" noise.
        let dir = tempfile::tempdir().unwrap();
        let personas = dir.path().join("personas.json");
        let teams = dir.path().join("teams.json");
        let r = commit_stores(
            &personas,
            &teams,
            || Err("write failed".to_string()),
            || unreachable!(),
        );
        assert!(r.is_err());
        let msg = r.unwrap_err();
        assert!(msg.contains("write failed"));
        assert!(!msg.contains("could not be restored"), "{msg}");
        assert!(!personas.exists() && !teams.exists());

        // Part B — independent restores: personas restore fails (dir gone after
        // the first write), teams restore is a no-op (absent snap → NotFound).
        // Both failures aggregated in the returned error (I5).
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let personas2 = sub.join("personas.json");
        let teams2 = sub.join("teams.json");
        write_file(&personas2, b"snap-p");
        let sub_clone = sub.clone();
        let r2 = commit_stores(
            &personas2,
            &teams2,
            || {
                fs::write(&personas2, b"new-p").map_err(|e| e.to_string())?;
                let _ = std::fs::remove_dir_all(&sub_clone);
                Ok(())
            },
            || Err("teams write failed".to_string()),
        );
        assert!(r2.is_err());
        let msg2 = r2.unwrap_err();
        assert!(msg2.contains("teams write failed"), "{msg2}");
        assert!(msg2.contains("could not be restored"), "{msg2}");
    }
}
