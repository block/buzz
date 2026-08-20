//! Tests for inbound persona/team/managed-agent reconciliation.
//! Extracted from the parent module to keep it under the file-size cap.

use super::*;
use std::collections::BTreeMap;

const UUID: &str = "11111111-2222-3333-4444-555555555555"; // sadscan:disable sq.pii.cc.visa -- fixed test UUID

/// A local in-app persona: `source_team_persona_slug` is None, so its d-tag
/// IS its UUID id. Carries env_vars + source_team that must survive a patch.
fn local_in_app() -> AgentDefinition {
    AgentDefinition {
        id: UUID.to_string(),
        display_name: "Local".to_string(),
        avatar_url: None,
        system_prompt: "local prompt".to_string(),
        runtime: Some("goose".to_string()),
        model: Some("opus".to_string()),
        provider: Some("anthropic".to_string()),
        name_pool: vec!["Local".to_string()],
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: Some("team-1".to_string()),
        source_team_persona_slug: None,
        catalog_source: None,
        env_vars: BTreeMap::from([("API_KEY".to_string(), "secret".to_string())]),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
    }
}

/// An inbound persona as `persona_from_event` would produce it: id = d-tag,
/// slug = Some(d-tag), empty env_vars, source_team None.
fn inbound_for(d_tag: &str, display_name: &str) -> AgentDefinition {
    AgentDefinition {
        id: d_tag.to_string(),
        display_name: display_name.to_string(),
        avatar_url: Some("https://example.com/a.png".to_string()),
        system_prompt: "remote prompt".to_string(),
        runtime: Some("acp".to_string()),
        model: Some("sonnet".to_string()),
        provider: Some("openai".to_string()),
        name_pool: vec!["Remote".to_string()],
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: Some(d_tag.to_string()),
        catalog_source: None,
        env_vars: BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2025-06-01T00:00:00Z".to_string(),
        updated_at: "2025-06-01T00:00:00Z".to_string(),
    }
}

#[test]
fn in_app_persona_matches_existing_uuid_and_patches() {
    let mut personas = vec![local_in_app()];
    apply_inbound_persona(&mut personas, inbound_for(UUID, "Remote"));

    assert_eq!(personas.len(), 1, "no duplicate row");
    let p = &personas[0];
    // Projected fields patched.
    assert_eq!(p.display_name, "Remote");
    assert_eq!(p.system_prompt, "remote prompt");
    assert_eq!(p.provider, Some("openai".to_string()));
    // Local identity + secrets + lineage preserved.
    assert_eq!(p.id, UUID);
    assert_eq!(p.env_vars.get("API_KEY"), Some(&"secret".to_string()));
    assert_eq!(p.source_team, Some("team-1".to_string()));
    assert_eq!(p.source_team_persona_slug, None);
    assert_eq!(p.created_at, "2025-01-01T00:00:00Z");
}

#[test]
fn inbound_quad_edit_applies_to_existing_matched_record() {
    // B5 quad activation: a remote quad edit must land on the MATCH branch,
    // not just the insert branch — otherwise device B keeps its stale quad
    // and its next reconcile republishes it over device A's edit, and the
    // two devices never converge (permanent ping-pong).
    let mut local = local_in_app();
    local.respond_to = Some("owner-only".to_string());
    local.parallelism = Some(2);
    let mut personas = vec![local];

    let mut inbound = inbound_for(UUID, "Remote");
    inbound.respond_to = Some("allowlist".to_string());
    inbound.respond_to_allowlist = vec!["a".repeat(64)];
    inbound.parallelism = Some(8);
    apply_inbound_persona(&mut personas, inbound);

    assert_eq!(personas.len(), 1, "no duplicate row");
    let p = &personas[0];
    assert_eq!(p.respond_to, Some("allowlist".to_string()));
    assert_eq!(p.respond_to_allowlist, vec!["a".repeat(64)]);
    assert_eq!(p.parallelism, Some(8));
    // A quad-absent inbound also applies (clears), same as prompt/model.
    apply_inbound_persona(&mut personas, inbound_for(UUID, "Remote"));
    assert_eq!(personas[0].respond_to, None);
    assert_eq!(personas[0].parallelism, None);
}

#[test]
fn re_received_in_app_persona_is_idempotent_no_duplicate() {
    let mut personas = vec![local_in_app()];
    apply_inbound_persona(&mut personas, inbound_for(UUID, "Remote"));
    // Same event arrives again (e.g. reconnect backfill).
    apply_inbound_persona(&mut personas, inbound_for(UUID, "Remote"));

    assert_eq!(personas.len(), 1, "re-receive must not duplicate");
    assert_eq!(personas[0].id, UUID);
}

#[test]
fn team_persona_matches_on_slug_and_patches() {
    let mut local = local_in_app();
    local.id = "local-uuid".to_string();
    local.source_team_persona_slug = Some("team-slug".to_string());
    let mut personas = vec![local];

    apply_inbound_persona(&mut personas, inbound_for("team-slug", "Renamed"));

    assert_eq!(personas.len(), 1, "no duplicate row");
    assert_eq!(personas[0].display_name, "Renamed");
    // Local UUID survives even though the match key is the slug.
    assert_eq!(personas[0].id, "local-uuid");
    assert_eq!(
        personas[0].source_team_persona_slug,
        Some("team-slug".to_string())
    );
}

#[test]
fn no_local_match_inserts_inbound_reusing_d_tag_as_id() {
    let mut personas = vec![local_in_app()];
    let other = "99999999-8888-7777-6666-555555555555";
    apply_inbound_persona(&mut personas, inbound_for(other, "New"));

    assert_eq!(personas.len(), 2, "unmatched inbound is inserted");
    let inserted = personas.iter().find(|p| p.id == other).unwrap();
    assert_eq!(inserted.display_name, "New");
    // Re-receiving the inserted record must still be idempotent.
    apply_inbound_persona(&mut personas, inbound_for(other, "New"));
    assert_eq!(personas.len(), 2, "re-receive of inserted record no-ops");
}

// ── Managed-agent (30177) inbound ────────────────────────────────────────

const AGENT_PUBKEY: &str = "agentpubkeyhex0000000000000000000000000000000000000000000000000000";

/// A local managed agent carrying every device-local secret that an inbound
/// event must NEVER be able to overwrite.
fn local_agent() -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: AGENT_PUBKEY.to_string(),
        name: "Local Agent".to_string(),
        persona_id: Some("persona-local".to_string()),
        private_key_nsec: "nsec1localsecret".to_string(),
        auth_tag: Some("localauthtag".to_string()),
        relay_url: "wss://relay.local".to_string(),
        avatar_url: None,
        acp_command: "buzz-acp".to_string(),
        agent_command: "goose".to_string(),
        agent_command_override: Some("claude".to_string()),
        agent_args: vec![],
        mcp_command: "buzz-dev-mcp".to_string(),
        turn_timeout_seconds: 320,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 8,
        system_prompt: Some("local prompt".to_string()),
        model: Some("local-model".to_string()),
        provider: Some("local-provider".to_string()),
        persona_source_version: Some("local-hash".to_string()),
        env_vars: BTreeMap::from([("API_KEY".to_string(), "localsecret".to_string())]),
        start_on_app_launch: true,
        auto_restart_on_config_change: true,
        runtime_pid: Some(1234),
        backend: crate::managed_agents::BackendKind::Provider {
            id: "buzz-backend".to_string(),
            config: serde_json::json!({ "api_key": "localproviderkey" }),
        },
        backend_agent_id: Some("local-remote-id".to_string()),
        provider_policy_pending: false,
        provider_binary_path: Some("/local/bin".to_string()),
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: crate::managed_agents::RespondTo::OwnerOnly,
        respond_to_allowlist: vec![],
        display_name: None,
        slug: None,
        runtime: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        library_ref: None,
        library_applied_revision: None,
        last_completed_deploy_attempt_id: None,
        definition_respond_to: None,
        definition_respond_to_allowlist: Vec::new(),
        definition_parallelism: None,
        relay_mesh: None,
        effort_level: None,
    }
}

/// Sign a kind:30177 event whose content JSON carries the legitimate
/// projected fields PLUS injected secret/harness keys — a hostile relay
/// event trying to smuggle credentials onto the apply path.
fn foreign_agent_event_with_secrets(d_tag: &str) -> nostr::Event {
    use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};
    let content = serde_json::json!({
        "name": "Remote Agent",
        "persona_id": "persona-remote",
        "system_prompt": "remote prompt",
        "model": "remote-model",
        "provider": "remote-provider",
        "persona_source_version": "remote-hash",
        "parallelism": 99,
        "respond_to": "anyone",
        "respond_to_allowlist": ["deadbeef"],
        // Injected — must be dropped at deserialization, never applied.
        "private_key_nsec": "nsec1INJECTEDSECRET",
        "auth_tag": "INJECTEDAUTHTAG",
        "env_vars": { "API_KEY": "INJECTEDKEY" },
        "agent_command": "INJECTEDHARNESS",
        "agent_command_override": "INJECTEDOVERRIDE",
        "backend": { "type": "provider", "id": "x", "config": { "k": "INJECTEDBACKEND" } },
        "mcp_command": "INJECTEDMCP",
    });
    let keys = Keys::generate();
    let event = EventBuilder::new(Kind::Custom(30177), content.to_string())
        .tags(vec![Tag::parse(["d", d_tag]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    // Round-trip through JSON to mirror the wire path the reconcile command
    // parses from.
    nostr::Event::from_json(event.as_json()).unwrap()
}

/// Direct-backend secret-preservation: drive the real parser + apply against
/// a foreign event crammed with secrets and assert NONE land on the local
/// record, and that every projected field IS updated. The projection type is
/// the structural guard — the injected keys cannot even be represented.
#[test]
fn inbound_managed_agent_drops_injected_secrets_and_harness() {
    let event = foreign_agent_event_with_secrets(AGENT_PUBKEY);
    let content =
        crate::managed_agents::agent_events::managed_agent_content_from_event(&event).unwrap();
    let mut agents = vec![local_agent()];
    let InboundAgentApply { access_changed, .. } =
        apply_inbound_managed_agent(&mut agents, &[], AGENT_PUBKEY, content);

    assert_eq!(
        access_changed,
        !crate::managed_agents::owner_only_access_build(),
        "only an effective access change may trigger a runtime refresh"
    );
    let a = &agents[0];
    // Secrets / harness / runtime — every one preserved from the local record.
    assert_eq!(
        a.private_key_nsec, "nsec1localsecret",
        "secret key overwritten"
    );
    assert_eq!(
        a.auth_tag,
        Some("localauthtag".to_string()),
        "auth tag overwritten"
    );
    assert_eq!(
        a.env_vars.get("API_KEY"),
        Some(&"localsecret".to_string()),
        "env var overwritten"
    );
    assert_eq!(a.agent_command, "goose", "harness command overwritten");
    assert_eq!(
        a.agent_command_override,
        Some("claude".to_string()),
        "harness override overwritten"
    );
    assert_eq!(a.mcp_command, "buzz-dev-mcp", "mcp command overwritten");
    assert_eq!(a.relay_url, "wss://relay.local", "relay url overwritten");
    assert_eq!(a.runtime_pid, Some(1234), "runtime pid overwritten");
    match &a.backend {
        crate::managed_agents::BackendKind::Provider { config, .. } => {
            assert_eq!(
                config["api_key"], "localproviderkey",
                "backend blob overwritten"
            );
        }
        _ => panic!("backend kind changed"),
    }
    // No injected value appears anywhere on the serialized record.
    let json = serde_json::to_string(a).unwrap();
    for needle in [
        "INJECTEDSECRET",
        "INJECTEDAUTHTAG",
        "INJECTEDKEY",
        "INJECTEDHARNESS",
        "INJECTEDOVERRIDE",
        "INJECTEDBACKEND",
        "INJECTEDMCP",
    ] {
        assert!(!json.contains(needle), "injected value leaked: {needle}");
    }
    // Instance-level projected fields ARE updated from the inbound event.
    assert_eq!(a.name, "Remote Agent");
    assert_eq!(a.parallelism, 99);
    assert_eq!(a.respond_to, crate::managed_agents::RespondTo::Anyone);
    assert_eq!(a.respond_to_allowlist, vec!["deadbeef".to_string()]);
    // Definition-linked inbound (persona_id present): the definition quad is
    // NOT applied — those fields resolve through the kind:30175 definition,
    // and absent-on-the-wire must never clear a local snapshot.
    assert_eq!(
        a.system_prompt,
        Some("local prompt".to_string()),
        "linked inbound must not touch the local prompt snapshot"
    );
}

/// Definition-less inbound (persona_id absent) still applies the definition
/// quad unconditionally — the record is its own definition and the wire is
/// its sync channel.
#[test]
fn inbound_definition_less_agent_applies_quad() {
    use nostr::{EventBuilder, Keys, Kind, Tag};
    // Same wire shape as the hostile fixture, minus persona_id — a
    // definition-less instance syncing its own definition fields.
    let content = serde_json::json!({
        "name": "Remote Agent",
        "system_prompt": "remote prompt",
        "model": "remote-model",
        "provider": "remote-provider",
        "persona_source_version": "remote-version",
        "parallelism": 99,
        "respond_to": "anyone",
        "respond_to_allowlist": ["deadbeef"],
    });
    let keys = Keys::generate();
    let event = EventBuilder::new(Kind::Custom(30177), content.to_string())
        .tags(vec![Tag::parse(["d", AGENT_PUBKEY]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();

    let content =
        crate::managed_agents::agent_events::managed_agent_content_from_event(&event).unwrap();
    let mut agents = vec![local_agent()];
    apply_inbound_managed_agent(&mut agents, &[], AGENT_PUBKEY, content);

    let a = &agents[0];
    assert_eq!(a.persona_id, None);
    assert_eq!(a.system_prompt, Some("remote prompt".to_string()));
    assert_eq!(a.model, Some("remote-model".to_string()));
    assert_eq!(a.provider, Some("remote-provider".to_string()));
    assert_eq!(
        a.persona_source_version,
        Some("remote-version".to_string()),
        "all four quad fields must apply on a definition-less sync"
    );
}

#[test]
fn inbound_managed_agent_no_match_is_noop() {
    let event = foreign_agent_event_with_secrets("someotheragentpubkey");
    let content =
        crate::managed_agents::agent_events::managed_agent_content_from_event(&event).unwrap();
    let mut agents = vec![local_agent()];
    apply_inbound_managed_agent(&mut agents, &[], "someotheragentpubkey", content);

    // No agent minted from a relay event — it would have no secret key.
    assert_eq!(agents.len(), 1);
    assert_eq!(
        agents[0].name, "Local Agent",
        "unmatched inbound must not touch the local record"
    );
}

// ── §2.8 canonical-linkage rule (kind:30177) ─────────────────────────────

/// A keyless definition (former persona) for the linkage resolver: `into_
/// agent_record` sets `slug = id`, and a projected one carries `library_ref`.
fn definition(slug: &str, projected: bool) -> ManagedAgentRecord {
    let mut record = inbound_for(slug, "Definition").into_agent_record();
    if projected {
        record.library_ref = Some(format!("lib-{slug}"));
        record.library_applied_revision = Some(1);
    }
    record
}

/// Inbound kind:30177 content carrying an explicit `persona_id` (or `None`).
/// Mirrors the wire shape `managed_agent_content_from_event` produces.
fn agent_content(name: &str, persona_id: Option<&str>) -> ManagedAgentEventContent {
    ManagedAgentEventContent {
        name: name.to_string(),
        persona_id: persona_id.map(str::to_string),
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        parallelism: 4,
        respond_to: crate::managed_agents::RespondTo::OwnerOnly,
        respond_to_allowlist: vec![],
    }
}

/// A local instance linked to `persona_id`, keyed by `AGENT_PUBKEY`.
fn linked_agent(persona_id: &str) -> ManagedAgentRecord {
    let mut agent = local_agent();
    agent.persona_id = Some(persona_id.to_string());
    agent
}

/// An inbound event that would re-point a library-owned linkage is frozen:
/// `persona_id` stays on the local library-projected definition, safe fields
/// still apply, and the caller is told to converge the relay head back (§2.8).
#[test]
fn inbound_30177_freezes_repoint_of_library_owned_linkage() {
    let definitions = vec![definition("shared-def", true)];
    let mut agents = vec![linked_agent("shared-def")];

    // Inbound tries to re-point the linkage to a different definition.
    let outcome = apply_inbound_managed_agent(
        &mut agents,
        &definitions,
        AGENT_PUBKEY,
        agent_content("Renamed", Some("other-def")),
    );

    assert_eq!(
        outcome.linkage,
        InboundAgentLinkage::Frozen(LinkageFreezeReason::OwnedByLibrary),
        "re-pointing a library-owned linkage must freeze",
    );
    let a = &agents[0];
    assert_eq!(
        a.persona_id,
        Some("shared-def".to_string()),
        "linkage must stay on the local library-owned definition",
    );
    assert_eq!(a.name, "Renamed", "safe per-instance fields still apply");
    assert_eq!(a.parallelism, 4, "safe per-instance fields still apply");
}

/// An inbound event that would CLEAR a library-owned linkage
/// (`persona_id: None`) is frozen the same way — clearing is authorship too.
#[test]
fn inbound_30177_freezes_clear_of_library_owned_linkage() {
    let definitions = vec![definition("shared-def", true)];
    let mut agents = vec![linked_agent("shared-def")];

    let outcome = apply_inbound_managed_agent(
        &mut agents,
        &definitions,
        AGENT_PUBKEY,
        agent_content("Renamed", None),
    );

    assert_eq!(
        outcome.linkage,
        InboundAgentLinkage::Frozen(LinkageFreezeReason::OwnedByLibrary),
    );
    assert_eq!(
        agents[0].persona_id,
        Some("shared-def".to_string()),
        "a definition-less inbound must not clear a library-owned linkage",
    );
}

/// An inbound event that would newly link a currently-plain instance to a
/// library-projected definition is frozen as an inadmissible new link — only
/// the Phase-4b coordinator may admit a projected link (§2.8, P6-C1 interim).
#[test]
fn inbound_30177_freezes_inadmissible_new_link_to_projected_definition() {
    let definitions = vec![definition("shared-def", true)];
    // Local instance is definition-less (plain).
    let mut agents = vec![linked_agent("")];
    agents[0].persona_id = None;

    let outcome = apply_inbound_managed_agent(
        &mut agents,
        &definitions,
        AGENT_PUBKEY,
        agent_content("Renamed", Some("shared-def")),
    );

    assert_eq!(
        outcome.linkage,
        InboundAgentLinkage::Frozen(LinkageFreezeReason::InadmissibleNewLink),
        "a new link to a projected definition must fail closed",
    );
    assert_eq!(
        agents[0].persona_id, None,
        "the inadmissible new link must not be authored",
    );
}

/// A linkage change that touches only plain definitions applies as at head —
/// the §2.8 rule freezes ONLY library-owned or projected-target changes, never
/// an ordinary plain relink.
#[test]
fn inbound_30177_applies_plain_relink_unchanged() {
    let definitions = vec![definition("plain-a", false), definition("plain-b", false)];
    let mut agents = vec![linked_agent("plain-a")];

    let outcome = apply_inbound_managed_agent(
        &mut agents,
        &definitions,
        AGENT_PUBKEY,
        agent_content("Renamed", Some("plain-b")),
    );

    assert_eq!(outcome.linkage, InboundAgentLinkage::Applied);
    assert_eq!(
        agents[0].persona_id,
        Some("plain-b".to_string()),
        "a plain→plain relink applies exactly as at head",
    );
}

/// An inbound event that leaves the linkage unchanged is never frozen even when
/// the linked definition is library-projected — the definition quad is still
/// correctly omitted (linked), and safe fields apply. Freezing keys on a
/// linkage CHANGE, not on the linked definition's library status alone.
#[test]
fn inbound_30177_no_linkage_change_applies_even_when_projected() {
    let definitions = vec![definition("shared-def", true)];
    let mut agents = vec![linked_agent("shared-def")];
    agents[0].system_prompt = Some("local prompt".to_string());

    let outcome = apply_inbound_managed_agent(
        &mut agents,
        &definitions,
        AGENT_PUBKEY,
        agent_content("Renamed", Some("shared-def")),
    );

    assert_eq!(outcome.linkage, InboundAgentLinkage::Applied);
    let a = &agents[0];
    assert_eq!(a.persona_id, Some("shared-def".to_string()));
    assert_eq!(a.name, "Renamed");
    assert_eq!(
        a.system_prompt,
        Some("local prompt".to_string()),
        "a linked inbound omits the definition quad — the local snapshot survives",
    );
}

// ── §2.8 convergence: corrective re-retain is not best-effort (P1-I1) ─────

/// The healthy convergence path re-retains the LOCAL record's authoritative
/// projection under its coordinate, queued for publish — this is the row that
/// makes the relay head converge back after a frozen inbound event was
/// retained.
#[test]
fn converge_frozen_linkage_re_retains_local_head_pending() {
    use crate::managed_agents::retention::{get_retained_event, open_retention_db};
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    let keys = nostr::Keys::generate();
    let agents = vec![local_agent()];

    converge_frozen_linkage(&conn, &keys, &agents, AGENT_PUBKEY).unwrap();

    let row = get_retained_event(
        &conn,
        buzz_core_pkg::kind::KIND_MANAGED_AGENT,
        &keys.public_key().to_hex(),
        AGENT_PUBKEY,
    )
    .unwrap()
    .expect("convergence must retain the local head");
    assert!(row.pending_sync, "corrective row must queue for publish");
    assert!(
        row.content.contains("Local Agent"),
        "retained row must be the local authoritative projection",
    );
}

/// A failed corrective re-retain MUST propagate as `Err`, never be swallowed.
/// The inbound event is already the retained head when this runs; if it were
/// swallowed the command would report success while a non-authoritative head
/// stayed retained (and replay is dead — the same event re-arriving is
/// `Skipped` at the equal-`created_at` guard). A connection with no
/// `persona_events` table models the retention write failing after the inbound
/// retain: `retain_agent_record`'s first query fails.
#[test]
fn converge_frozen_linkage_errs_when_retain_fails() {
    let poisoned = rusqlite::Connection::open_in_memory().unwrap();
    let keys = nostr::Keys::generate();
    let agents = vec![local_agent()];

    let err = converge_frozen_linkage(&poisoned, &keys, &agents, AGENT_PUBKEY).unwrap_err();
    assert!(
        err.contains("convergence"),
        "a failed corrective re-retain must surface as an error: {err}"
    );
}

/// A frozen classification with no matching local record is an internal
/// inconsistency (the freeze was decided against a record that must exist) and
/// fails closed rather than silently no-oping the convergence.
#[test]
fn converge_frozen_linkage_errs_when_record_missing() {
    use crate::managed_agents::retention::open_retention_db;
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    let keys = nostr::Keys::generate();

    let err = converge_frozen_linkage(&conn, &keys, &[], AGENT_PUBKEY).unwrap_err();
    assert!(
        err.contains("not found"),
        "a missing frozen record must fail closed: {err}"
    );
}

// Team (kind:30176) inbound tests — split to keep this file under the cap.
#[path = "team_tests.rs"]
mod team_tests;
use team_tests::{local_team, TEAM_ID};

// ── Tombstone (kind:5) consume ────────────────────────────────────────────

fn deletion_event(coord: &str) -> nostr::Event {
    deletion_event_with_keys(coord, &nostr::Keys::generate())
}

fn deletion_event_with_keys(coord: &str, keys: &nostr::Keys) -> nostr::Event {
    use nostr::{EventBuilder, JsonUtil, Kind, Tag};
    let event = EventBuilder::new(Kind::Custom(5), "")
        .tags(vec![Tag::parse(["a", coord]).unwrap()])
        .sign_with_keys(keys)
        .unwrap();
    nostr::Event::from_json(event.as_json()).unwrap()
}

/// A deletion event whose coordinate owner IS its signer — the only shape
/// `parse_deletion_coordinate` accepts since the owner check landed.
fn owned_deletion_event(kind: u32, d_tag: &str) -> nostr::Event {
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    deletion_event_with_keys(&format!("{kind}:{owner}:{d_tag}"), &keys)
}

#[test]
fn parse_deletion_coordinate_extracts_kind_and_d_tag() {
    // Persona / team / agent coordinates all route by their leading kind.
    let p = owned_deletion_event(30175, "my-persona");
    assert_eq!(
        parse_deletion_coordinate(&p),
        Some((30175, "my-persona".to_string()))
    );
    let a = owned_deletion_event(30177, "agentpubkeyhex");
    assert_eq!(
        parse_deletion_coordinate(&a),
        Some((30177, "agentpubkeyhex".to_string()))
    );
}

#[test]
fn parse_deletion_coordinate_rejects_foreign_owner() {
    // A validly signed kind:5 naming ANOTHER owner's coordinate must no-op:
    // NIP-09 scopes deletion to the record's own author.
    let foreign_owner = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    let forged = deletion_event(&format!("30175:{foreign_owner}:my-persona"));
    assert_eq!(parse_deletion_coordinate(&forged), None);
}

#[test]
fn parse_deletion_coordinate_handles_colon_in_d_tag_and_rejects_malformed() {
    // A d-tag containing ':' keeps its remainder intact (splitn(3)).
    let weird = owned_deletion_event(30176, "a:b:c");
    assert_eq!(
        parse_deletion_coordinate(&weird),
        Some((30176, "a:b:c".to_string()))
    );
    // Missing d-tag segment / non-numeric kind → None (no-op).
    assert_eq!(
        parse_deletion_coordinate(&deletion_event("30175:owner")),
        None
    );
    assert_eq!(
        parse_deletion_coordinate(&deletion_event("notakind:owner:d")),
        None
    );
}

#[test]
fn tombstone_removal_predicates_match_apply_fn_keys() {
    // The deletion path removes by the SAME per-kind key the apply fns use.
    // Persona: by persona_d_tag (slug/id).
    let mut personas = vec![local_in_app()];
    let target = persona_d_tag(&personas[0]);
    personas.retain(|r| persona_d_tag(r) != target);
    assert!(personas.is_empty(), "persona removed by its d-tag");

    // Team: by id.
    let mut teams = vec![local_team()];
    teams.retain(|r| r.id != TEAM_ID);
    assert!(teams.is_empty(), "team removed by id");

    // Managed-agent: by pubkey. A non-matching d-tag is a no-op.
    let mut agents = vec![local_agent()];
    agents.retain(|r| r.pubkey != "someoneelse");
    assert_eq!(agents.len(), 1, "non-matching agent tombstone no-ops");
    agents.retain(|r| r.pubkey != AGENT_PUBKEY);
    assert!(agents.is_empty(), "agent removed by pubkey");
}

// ── Inbound signature gate ──────────────────────────────────────────────────

#[test]
fn inbound_gate_rejects_tampered_event() {
    use nostr::JsonUtil;
    // A validly signed event whose content was altered post-signing: the
    // pubkey is real, the sig no longer covers the bytes. Must die at the
    // gate before any store logic runs.
    let keys = nostr::Keys::generate();
    let event = nostr::EventBuilder::new(nostr::Kind::Custom(30175), "{}")
        .tags(vec![nostr::Tag::parse(["d", "victim-slug"]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let tampered = event.as_json().replace(
        "\"content\":\"{}\"",
        "\"content\":\"{\\\"system_prompt\\\":\\\"pwned\\\"}\"",
    );
    assert_ne!(
        tampered,
        event.as_json(),
        "string replace must have taken effect — if this fails the test is testing an un-tampered event"
    );

    let err = parse_verified_inbound_event(&tampered).unwrap_err();
    assert!(
        err.contains("signature"),
        "tampered event must fail the signature gate: {err}"
    );
}

#[test]
fn inbound_gate_accepts_validly_signed_event() {
    use nostr::JsonUtil;
    let keys = nostr::Keys::generate();
    let event = nostr::EventBuilder::new(nostr::Kind::Custom(30175), "{}")
        .tags(vec![nostr::Tag::parse(["d", "slug"]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let parsed = parse_verified_inbound_event(&event.as_json()).unwrap();
    assert_eq!(parsed.pubkey, keys.public_key());
}

// The command-seam integration fixture (P2-I1) lives in a sibling file to keep
// this module under the 1000-line file-size cap. Included here so it shares the
// fixtures above via `use super::*`.
#[path = "seam_tests.rs"]
mod seam_tests;

#[test]
fn inbound_persona_rejects_invisible_definition_text() {
    let mut inbound = inbound_for("unsafe", "Remote");
    inbound.system_prompt = "Review\u{200B} code.".to_string();

    let error = validate_inbound_persona_definition(&inbound)
        .expect_err("relay sync must reject invisible instructions");

    assert!(error.contains("U+200B"));
}

fn inbound_managed_agent_content(
    name: &str,
    persona_id: Option<&str>,
    system_prompt: Option<&str>,
) -> crate::managed_agents::agent_events::ManagedAgentEventContent {
    crate::managed_agents::agent_events::ManagedAgentEventContent {
        name: name.to_string(),
        persona_id: persona_id.map(str::to_string),
        system_prompt: system_prompt.map(str::to_string),
        model: None,
        provider: None,
        persona_source_version: None,
        parallelism: 1,
        respond_to: crate::managed_agents::RespondTo::OwnerOnly,
        respond_to_allowlist: vec![],
    }
}

#[test]
fn inbound_definition_less_agent_rejects_invisible_prompt() {
    let inbound = inbound_managed_agent_content("Remote Agent", None, Some("Review\u{200B} code."));

    let error = validate_inbound_managed_agent_definition(&inbound)
        .expect_err("definition-less sync must reject invisible instructions");

    assert!(error.contains("U+200B"));
}

#[test]
fn inbound_managed_agent_rejects_bidirectional_name() {
    let inbound = inbound_managed_agent_content("Remote\u{202E} Agent", None, None);

    let error = validate_inbound_managed_agent_definition(&inbound)
        .expect_err("managed-agent sync must reject bidirectional names");

    assert!(error.contains("U+202E"));
}

#[test]
fn inbound_definition_less_agent_accepts_visible_multiline_prompt() {
    let inbound = inbound_managed_agent_content(
        "Remote Agent",
        None,
        Some("Review code.\n\tCall out security risks."),
    );

    assert!(validate_inbound_managed_agent_definition(&inbound).is_ok());
}
