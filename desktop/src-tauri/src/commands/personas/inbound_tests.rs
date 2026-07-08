//! Tests for inbound persona/team/managed-agent reconciliation.
//! Extracted from the parent module to keep it under the file-size cap.

use super::*;
use std::collections::BTreeMap;

const UUID: &str = "11111111-2222-3333-4444-555555555555";

/// A local in-app persona: `source_team_persona_slug` is None, so its d-tag
/// IS its UUID id. Carries env_vars + source_team that must survive a patch.
fn local_in_app() -> PersonaRecord {
    PersonaRecord {
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
        source_team: Some("team-1".to_string()),
        source_team_persona_slug: None,
        env_vars: BTreeMap::from([("API_KEY".to_string(), "secret".to_string())]),
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
    }
}

/// An inbound persona as `persona_from_event` would produce it: id = d-tag,
/// slug = Some(d-tag), empty env_vars, source_team None.
fn inbound_for(d_tag: &str, display_name: &str) -> PersonaRecord {
    PersonaRecord {
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
        source_team: None,
        source_team_persona_slug: Some(d_tag.to_string()),
        env_vars: BTreeMap::new(),
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
        mcp_toolsets: Some("local".to_string()),
        env_vars: BTreeMap::from([("API_KEY".to_string(), "localsecret".to_string())]),
        start_on_app_launch: true,
        auto_restart_on_config_change: true,
        runtime_pid: Some(1234),
        backend: crate::managed_agents::BackendKind::Provider {
            id: "buzz-backend".to_string(),
            config: serde_json::json!({ "api_key": "localproviderkey" }),
        },
        backend_agent_id: Some("local-remote-id".to_string()),
        provider_binary_path: Some("/local/bin".to_string()),
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        respond_to: crate::managed_agents::RespondTo::OwnerOnly,
        respond_to_allowlist: vec![],
        display_name: None,
        slug: None,
        runtime: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        relay_mesh: None,
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
        "mcp_toolsets": "remote",
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
    apply_inbound_managed_agent(&mut agents, AGENT_PUBKEY, content);

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
        "mcp_toolsets": "remote",
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
    apply_inbound_managed_agent(&mut agents, AGENT_PUBKEY, content);

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
    apply_inbound_managed_agent(&mut agents, "someotheragentpubkey", content);

    // No agent minted from a relay event — it would have no secret key.
    assert_eq!(agents.len(), 1);
    assert_eq!(
        agents[0].name, "Local Agent",
        "unmatched inbound must not touch the local record"
    );
}

// ── Team (30176) inbound ─────────────────────────────────────────────────

const TEAM_ID: &str = "team-local-id";

fn local_team() -> TeamRecord {
    TeamRecord {
        id: TEAM_ID.to_string(),
        name: "Local Team".to_string(),
        description: Some("local desc".to_string()),
        persona_ids: vec!["p-local".to_string()],
        is_builtin: false,
        source_dir: Some(std::path::PathBuf::from("/local/team/dir")),
        is_symlink: true,
        symlink_target: Some("/external".to_string()),
        version: Some("1.0".to_string()),
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
    }
}

fn team_content(name: &str) -> TeamEventContent {
    TeamEventContent {
        name: name.to_string(),
        description: Some("remote desc".to_string()),
        persona_ids: vec!["p-remote-1".to_string(), "p-remote-2".to_string()],
    }
}

#[test]
fn inbound_team_match_patches_shared_preserves_local() {
    let mut teams = vec![local_team()];
    apply_inbound_team(
        &mut teams,
        TEAM_ID.to_string(),
        team_content("Renamed Team"),
    );

    assert_eq!(teams.len(), 1, "no duplicate row");
    let t = &teams[0];
    // Shared fields overwritten.
    assert_eq!(t.name, "Renamed Team");
    assert_eq!(t.description, Some("remote desc".to_string()));
    assert_eq!(
        t.persona_ids,
        vec!["p-remote-1".to_string(), "p-remote-2".to_string()]
    );
    // Install-local fields preserved.
    assert_eq!(t.id, TEAM_ID);
    assert_eq!(
        t.source_dir,
        Some(std::path::PathBuf::from("/local/team/dir"))
    );
    assert!(t.is_symlink);
    assert_eq!(t.symlink_target, Some("/external".to_string()));
    assert_eq!(t.version, Some("1.0".to_string()));
    assert_eq!(t.created_at, "2025-01-01T00:00:00Z");
}

#[test]
fn inbound_team_no_match_inserts_idempotently() {
    let mut teams = vec![local_team()];
    let other = "team-remote-id";
    apply_inbound_team(&mut teams, other.to_string(), team_content("New Team"));

    assert_eq!(teams.len(), 2, "unmatched inbound is inserted");
    let inserted = teams.iter().find(|t| t.id == other).unwrap();
    assert_eq!(inserted.name, "New Team");
    assert!(
        inserted.source_dir.is_none(),
        "inserted team has no local install dir"
    );
    // Re-receive stays idempotent.
    apply_inbound_team(&mut teams, other.to_string(), team_content("New Team"));
    assert_eq!(teams.len(), 2, "re-receive of inserted team no-ops");
}

// ── Tombstone (kind:5) consume ────────────────────────────────────────────

fn deletion_event(coord: &str) -> nostr::Event {
    use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};
    let event = EventBuilder::new(Kind::Custom(5), "")
        .tags(vec![Tag::parse(["a", coord]).unwrap()])
        .sign_with_keys(&Keys::generate())
        .unwrap();
    nostr::Event::from_json(event.as_json()).unwrap()
}

#[test]
fn parse_deletion_coordinate_extracts_kind_and_d_tag() {
    let owner = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    // Persona / team / agent coordinates all route by their leading kind.
    let p = deletion_event(&format!("30175:{owner}:my-persona"));
    assert_eq!(
        parse_deletion_coordinate(&p),
        Some((30175, "my-persona".to_string()))
    );
    let a = deletion_event(&format!("30177:{owner}:agentpubkeyhex"));
    assert_eq!(
        parse_deletion_coordinate(&a),
        Some((30177, "agentpubkeyhex".to_string()))
    );
}

#[test]
fn parse_deletion_coordinate_handles_colon_in_d_tag_and_rejects_malformed() {
    let owner = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    // A d-tag containing ':' keeps its remainder intact (splitn(3)).
    let weird = deletion_event(&format!("30176:{owner}:a:b:c"));
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
