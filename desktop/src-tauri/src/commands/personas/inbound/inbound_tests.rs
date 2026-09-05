//! Tests for inbound persona/team/managed-agent reconciliation.
//! Extracted from the parent module to keep it under the file-size cap.

use super::*;
use nostr::{JsonUtil, ToBech32};
use std::collections::BTreeMap;

const UUID: &str = "11111111-2222-3333-4444-555555555555"; // sadscan:disable sq.pii.cc.visa -- fixed test UUID

/// A local in-app persona: `source_team_persona_slug` is None, so its d-tag
/// IS its UUID id. Carries env_vars + source_team that must survive a patch.
fn local_in_app() -> AgentDefinition {
    AgentDefinition {
        description: None,
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
        team_catalog_source: None,
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
        description: None,
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
        team_catalog_source: None,
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

fn private_agent_payload(
    owner_keys: &nostr::Keys,
    agent_keys: &nostr::Keys,
    name: &str,
    parallelism: u32,
) -> buzz_core_pkg::private_managed_agent::Payload {
    use buzz_core_pkg::private_managed_agent::{
        Payload, PrivateConfig, PrivateIdentity, FORMAT, VERSION,
    };

    Payload {
        format: FORMAT.into(),
        version: VERSION,
        agent_pubkey: agent_keys.public_key().to_hex(),
        owner_pubkey: owner_keys.public_key().to_hex(),
        generation: 1,
        previous_event_id: None,
        updated_at: "2026-08-06T00:00:00Z".into(),
        identity: PrivateIdentity {
            private_key_nsec: agent_keys.secret_key().to_bech32().unwrap(),
            auth_tag: None,
        },
        config: PrivateConfig {
            relay_url: "wss://relay.example".into(),
            name: name.into(),
            persona_id: None,
            runtime: Some("goose".into()),
            model: None,
            provider: None,
            system_prompt: Some("relay prompt".into()),
            parallelism: Some(parallelism),
            respond_to: None,
            respond_to_allowlist: vec![],
            agent_command_override: None,
            agent_args: vec![],
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            env_vars: BTreeMap::new(),
            backend: serde_json::json!({"type":"local"}),
            backend_agent_id: None,
            team_id: None,
            persona_name_in_team: None,
            relay_mesh: None,
            effort_level: None,
            extra: serde_json::Map::new(),
        },
        extensions: BTreeMap::new(),
        extra: serde_json::Map::new(),
    }
}

#[test]
fn private_agent_inbound_rejects_before_retain_and_stale_event_preserves_overlay() {
    use crate::managed_agents::{
        private_config_overlay::PrivateConfigOverlay,
        retention::{get_retained_event, open_retention_db, InboundOutcome},
    };
    use buzz_core_pkg::{kind::KIND_PRIVATE_MANAGED_AGENT, private_managed_agent};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();
    let mut overlay = PrivateConfigOverlay::default();

    let valid = private_agent_payload(&owner_keys, &agent_keys, "new", 4);
    let newer_event = private_managed_agent::build_event(&owner_keys, &valid, 20).unwrap();
    assert_eq!(
        apply_inbound_private_managed_agent_event(&newer_event, &owner_keys, &conn, &mut overlay,)
            .unwrap(),
        InboundOutcome::Applied
    );
    assert_eq!(overlay.resolved_records(&[])[0].name, "new");

    let mut malformed = private_agent_payload(&owner_keys, &agent_keys, "malformed", 4);
    malformed.generation = 2;
    malformed.previous_event_id = Some(newer_event.id.to_hex());
    malformed.config.backend = serde_json::json!({"type":"provider"});
    let malformed_event = private_managed_agent::build_event(&owner_keys, &malformed, 30).unwrap();
    assert!(apply_inbound_private_managed_agent_event(
        &malformed_event,
        &owner_keys,
        &conn,
        &mut overlay,
    )
    .is_err());
    assert_eq!(overlay.resolved_records(&[])[0].name, "new");
    let retained = get_retained_event(
        &conn,
        KIND_PRIVATE_MANAGED_AGENT,
        &owner_keys.public_key().to_hex(),
        &pubkey,
    )
    .unwrap()
    .unwrap();
    assert_eq!(retained.raw_event, newer_event.as_json());

    let stale = private_agent_payload(&owner_keys, &agent_keys, "stale", 2);
    let stale_event = private_managed_agent::build_event(&owner_keys, &stale, 10).unwrap();
    assert_eq!(
        apply_inbound_private_managed_agent_event(&stale_event, &owner_keys, &conn, &mut overlay,)
            .unwrap(),
        InboundOutcome::Skipped
    );
    assert_eq!(overlay.resolved_records(&[])[0].name, "new");
}

/// SAMI PROBE: the retention DB survives a restart but the overlay does not.
/// On the next launch the backfill re-delivers the SAME event, which resolves
/// to `Skipped` against the retained row — so `insert_patch` never runs and the
/// overlay stays empty for the whole session.
#[test]
fn sami_probe_overlay_does_not_rehydrate_after_restart() {
    use crate::managed_agents::{
        private_config_overlay::PrivateConfigOverlay,
        retention::{open_retention_db, InboundOutcome},
    };
    use buzz_core_pkg::private_managed_agent;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("retention.db");
    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();

    let payload = private_agent_payload(&owner_keys, &agent_keys, "relay name", 4);
    let event = private_managed_agent::build_event(&owner_keys, &payload, 20).unwrap();

    // ── Session 1: event arrives, overlay hydrates. ──
    {
        let conn = open_retention_db(&db_path).unwrap();
        let mut overlay = PrivateConfigOverlay::default();
        assert_eq!(
            apply_inbound_private_managed_agent_event(&event, &owner_keys, &conn, &mut overlay)
                .unwrap(),
            InboundOutcome::Applied
        );
        assert_eq!(
            overlay.resolved_records(&[]).len(),
            1,
            "control: overlay hydrates on first arrival"
        );
    }

    // ── Session 2: same DB file, fresh in-memory overlay (app restart). ──
    let conn = open_retention_db(&db_path).unwrap();
    let mut overlay = PrivateConfigOverlay::default();
    let outcome =
        apply_inbound_private_managed_agent_event(&event, &owner_keys, &conn, &mut overlay)
            .unwrap();
    assert_eq!(
        outcome,
        InboundOutcome::Skipped,
        "re-delivered event is deduped against the retained row"
    );
    assert!(
        overlay.resolved_records(&[]).is_empty(),
        "DEFECT: overlay is empty after restart — relay config silently unavailable"
    );

    // ── Positive control: the probe CAN observe hydration in session 2. ──
    // A strictly-newer event is the only thing that repopulates the overlay.
    let mut newer = private_agent_payload(&owner_keys, &agent_keys, "newer name", 4);
    newer.generation = 2;
    newer.previous_event_id = Some(event.id.to_hex());
    let newer_event = private_managed_agent::build_event(&owner_keys, &newer, 30).unwrap();
    assert_eq!(
        apply_inbound_private_managed_agent_event(&newer_event, &owner_keys, &conn, &mut overlay)
            .unwrap(),
        InboundOutcome::Applied
    );
    assert_eq!(
        overlay.resolved_records(&[])[0].name,
        "newer name",
        "positive control: this harness observes hydration when it happens"
    );
}

/// A local managed agent carrying every device-local secret that an inbound
/// event must NEVER be able to overwrite.
fn local_agent() -> ManagedAgentRecord {
    ManagedAgentRecord {
        description: None,
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
        team_catalog_source: None,
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
    let access_changed = apply_inbound_managed_agent(&mut agents, AGENT_PUBKEY, content);

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
        instructions: None,
        persona_ids: vec!["p-local".to_string()],
        is_builtin: false,
        shared: false,
        catalog_source: None,
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
        instructions: Some(Some("remote instructions".to_string())),
        persona_ids: Some(vec!["p-remote-1".to_string(), "p-remote-2".to_string()]),
    }
}

/// An inbound event shaped like one from a client that predates
/// always-publish: `instructions`/`persona_ids` both omitted (`None`).
fn team_content_omitting_optional_fields(name: &str) -> TeamEventContent {
    TeamEventContent {
        name: name.to_string(),
        description: Some("remote desc".to_string()),
        instructions: None,
        persona_ids: None,
    }
}

/// An inbound event that explicitly clears both fields: `instructions` is
/// `Some(None)` (JSON `null`), `persona_ids` is `Some(vec![])`.
fn team_content_clearing_optional_fields(name: &str) -> TeamEventContent {
    TeamEventContent {
        name: name.to_string(),
        description: Some("remote desc".to_string()),
        instructions: Some(None),
        persona_ids: Some(vec![]),
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
    assert_eq!(t.instructions, Some("remote instructions".to_string()));
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
fn inbound_team_omitted_fields_preserve_local() {
    // A `None` for instructions/persona_ids means the publisher predates
    // always-publish — its true value is unknown, so reconcile must
    // preserve whatever this device already has. This is the fix for the
    // Sietch Tabr wipe: an old-shaped (or genuinely field-omitting) event
    // must not blank out a team that has real membership/instructions.
    let mut teams = vec![local_team()];
    // Give local_team real instructions so preservation is discriminating:
    // the pre-fix blind-overwrite bug would collapse this to `None`, while
    // the fix must leave it untouched on an omitted field.
    teams[0].instructions = Some("local instructions".to_string());
    apply_inbound_team(
        &mut teams,
        TEAM_ID.to_string(),
        team_content_omitting_optional_fields("Renamed Team"),
    );

    assert_eq!(teams.len(), 1);
    let t = &teams[0];
    assert_eq!(
        t.name, "Renamed Team",
        "shared non-optional field still overwrites"
    );
    assert_eq!(
        t.instructions,
        Some("local instructions".to_string()),
        "omitted instructions preserves local value rather than wiping it"
    );
    assert_eq!(
        t.persona_ids,
        vec!["p-local".to_string()],
        "omitted persona_ids preserves local membership rather than wiping it"
    );
}

#[test]
fn inbound_team_explicit_clear_overwrites_local() {
    // `Some(None)` / `Some(vec![])` are the explicit-clear signals a
    // pre-fix client can never produce — these must still overwrite local.
    let mut teams = vec![local_team()];
    // Give local_team real instructions so the clear has something to erase.
    teams[0].instructions = Some("local instructions".to_string());

    apply_inbound_team(
        &mut teams,
        TEAM_ID.to_string(),
        team_content_clearing_optional_fields("Cleared Team"),
    );

    assert_eq!(teams.len(), 1);
    let t = &teams[0];
    assert_eq!(t.instructions, None, "explicit null clears instructions");
    assert_eq!(
        t.persona_ids,
        Vec::<String>::new(),
        "explicit empty array clears membership"
    );
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

// ── Inbound team → membership propagation (commit_inbound_team wiring) ─────

use std::cell::RefCell;

/// A running instance of `persona_id`, optionally bound to a team.
fn team_instance(seed: char, persona_id: &str, team_id: Option<&str>) -> ManagedAgentRecord {
    let mut record = local_agent();
    record.pubkey = seed.to_string().repeat(64);
    record.name = persona_id.to_string();
    record.persona_id = Some(persona_id.to_string());
    record.team_id = team_id.map(str::to_string);
    record
}

/// An inbound team edit that ADDS a persona must bind that persona's unbound
/// running instances to the team — exactly like a local `update_team`. Without
/// the propagation wiring the instance stays unbound (member in roster, not in
/// behavior) until restart.
#[test]
fn inbound_team_add_binds_unbound_instance_through_wiring() {
    let mut teams = vec![local_team()];
    teams[0].persona_ids = vec!["p-existing".to_string()];
    let existing = vec![
        team_instance('a', "p-added", None),
        team_instance('b', "p-existing", Some(TEAM_ID)),
    ];
    let saved = RefCell::new(None);

    commit_inbound_team(
        &mut teams,
        TEAM_ID.to_string(),
        TeamEventContent {
            name: "Team".to_string(),
            description: None,
            instructions: None,
            persona_ids: Some(vec!["p-existing".to_string(), "p-added".to_string()]),
        },
        |_| Ok(()),
        || Ok(existing.clone()),
        |records| {
            *saved.borrow_mut() = Some(records.to_vec());
            Ok(())
        },
    )
    .expect("inbound add succeeds");

    let saved = saved
        .borrow()
        .clone()
        .expect("add must save the agent store");
    assert_eq!(
        saved[0].team_id.as_deref(),
        Some(TEAM_ID),
        "the added persona's unbound instance is bound to the team"
    );
    assert_eq!(
        saved[1].team_id.as_deref(),
        Some(TEAM_ID),
        "an instance already on the team is untouched"
    );
}

/// An inbound team edit that REMOVES a persona ("keep agents") must detach that
/// persona's instances bound to this team, so a kept instance stops drawing the
/// team's instructions at spawn.
#[test]
fn inbound_team_removal_detaches_instance_through_wiring() {
    let mut teams = vec![local_team()];
    teams[0].persona_ids = vec!["p-removed".to_string()];
    let existing = vec![team_instance('a', "p-removed", Some(TEAM_ID))];
    let saved = RefCell::new(None);

    commit_inbound_team(
        &mut teams,
        TEAM_ID.to_string(),
        TeamEventContent {
            name: "Team".to_string(),
            description: None,
            instructions: None,
            persona_ids: Some(vec![]),
        },
        |_| Ok(()),
        || Ok(existing.clone()),
        |records| {
            *saved.borrow_mut() = Some(records.to_vec());
            Ok(())
        },
    )
    .expect("inbound removal succeeds");

    let saved = saved
        .borrow()
        .clone()
        .expect("removal must save the agent store");
    assert_eq!(
        saved[0].team_id, None,
        "the removed persona's instance is detached from the team"
    );
}

/// An inbound edit that omits `persona_ids` (a pre-always-publish client)
/// preserves local membership, so the delta is empty and no instance is
/// re-pointed — a metadata-only inbound edit must not disturb bindings.
#[test]
fn inbound_team_omitted_roster_leaves_bindings_untouched() {
    let mut teams = vec![local_team()];
    teams[0].persona_ids = vec!["p-a".to_string()];
    let existing = vec![team_instance('a', "p-a", None)];
    let saved = RefCell::new(None);

    commit_inbound_team(
        &mut teams,
        TEAM_ID.to_string(),
        team_content_omitting_optional_fields("Renamed"),
        |_| Ok(()),
        || Ok(existing.clone()),
        |records| {
            *saved.borrow_mut() = Some(records.to_vec());
            Ok(())
        },
    )
    .expect("inbound metadata-only edit succeeds");

    assert!(
        saved.borrow().is_none(),
        "an empty membership delta writes nothing to the agent store"
    );
}

/// A failing agent-store write after the authoritative `save_teams` is
/// swallowed: the inbound reconcile still succeeds (boot repair is the retry),
/// so a secondary-store hiccup never aborts an inbound event whose team write
/// already landed.
#[test]
fn inbound_team_swallows_agent_store_failure() {
    let mut teams = vec![local_team()];
    teams[0].persona_ids = vec![];
    commit_inbound_team(
        &mut teams,
        TEAM_ID.to_string(),
        TeamEventContent {
            name: "Team".to_string(),
            description: None,
            instructions: None,
            persona_ids: Some(vec!["p-added".to_string()]),
        },
        |_| Ok(()),
        || Err("agent store unreadable".to_string()),
        |_| Ok(()),
    )
    .expect("inbound reconcile swallows secondary-store failure");
}

/// A `persist_teams` error propagates — the authoritative team write failing is
/// a real reconcile failure, unlike best-effort agent IO.
#[test]
fn inbound_team_propagates_persist_teams_error() {
    let mut teams = vec![local_team()];
    let err = commit_inbound_team(
        &mut teams,
        TEAM_ID.to_string(),
        team_content("Team"),
        |_| Err("disk full".to_string()),
        || Ok(vec![]),
        |_| Ok(()),
    )
    .expect_err("a failed team persist must propagate");
    assert_eq!(err, "disk full");
}

// Tombstone authorization, signature-gate, and restart-rehydration
// regressions live in a sibling file to stay under the file-size cap.
#[path = "inbound_security_tests.rs"]
mod security_tests;

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
