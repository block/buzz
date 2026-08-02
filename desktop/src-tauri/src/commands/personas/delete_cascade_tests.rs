//! Tests for cascade-delete filtering in `delete_persona`.
//!
//! `delete_persona` deletes all managed-agent records whose `persona_id`
//! matches the persona being deleted, mirroring the cleanup done by
//! `delete_managed_agent`. These tests verify the `collect_cascade_pubkeys`
//! helper that identifies the agents to cascade-delete, using plain
//! in-memory data structures (no `AppHandle` required).

use super::{
    collect_cascade_pubkeys, collect_remote_deployed, commit_cascade_agents,
    preflight_remote_deployed_cascade,
};
use crate::managed_agents::{BackendKind, ManagedAgentRecord, RespondTo};
use std::collections::BTreeMap;
use std::collections::HashSet;

fn make_agent(
    pubkey: &str,
    persona_id: Option<&str>,
    runtime_pid: Option<u32>,
) -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: pubkey.to_string(),
        name: "Test Agent".to_string(),
        persona_id: persona_id.map(str::to_string),
        private_key_nsec: "".to_string(),
        auth_tag: None,
        relay_url: "ws://localhost:3000".to_string(),
        avatar_url: None,
        acp_command: "buzz-acp".to_string(),
        agent_command: "buzz-agent".to_string(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: "".to_string(),
        turn_timeout_seconds: 300,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        env_vars: BTreeMap::new(),
        start_on_app_launch: false,
        runtime_pid,
        backend: BackendKind::Local,
        backend_agent_id: None,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: RespondTo::OwnerOnly,
        respond_to_allowlist: vec![],
        display_name: None,
        slug: None,
        runtime: None,
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        relay_mesh: None,
        auto_restart_on_config_change: false,
        definition_respond_to: None,
        definition_respond_to_allowlist: vec![],
        definition_parallelism: None,
    }
}

const PERSONA_ID: &str = "custom:test-persona";

/// Deleting a persona with two linked agents (one running) returns both their
/// pubkeys and leaves unlinked agents out of the cascade set.
#[test]
fn cascade_includes_linked_agents_and_excludes_others() {
    let agents = vec![
        make_agent("agent-a", Some(PERSONA_ID), Some(12345)), // running, linked
        make_agent("agent-b", Some(PERSONA_ID), None),        // stopped, linked
        make_agent("agent-c", Some("custom:other"), None),    // different persona
        make_agent("agent-d", None, None),                    // no persona
    ];

    let pubkeys = collect_cascade_pubkeys(&agents, PERSONA_ID);

    assert_eq!(
        pubkeys.len(),
        2,
        "exactly two agents linked to this persona"
    );
    assert!(
        pubkeys.contains(&"agent-a".to_string()),
        "running linked agent included"
    );
    assert!(
        pubkeys.contains(&"agent-b".to_string()),
        "stopped linked agent included"
    );
    assert!(
        !pubkeys.contains(&"agent-c".to_string()),
        "different-persona agent excluded"
    );
    assert!(
        !pubkeys.contains(&"agent-d".to_string()),
        "persona-less agent excluded"
    );
}

/// Deleting a persona with no linked agents returns an empty list (no cascade).
#[test]
fn cascade_empty_when_no_linked_agents() {
    let agents = vec![
        make_agent("agent-x", Some("custom:other"), None),
        make_agent("agent-y", None, None),
    ];

    let pubkeys = collect_cascade_pubkeys(&agents, PERSONA_ID);

    assert!(pubkeys.is_empty(), "no agents to cascade-delete");
}

/// Cascade targets all agents linked to the persona — not just stopped ones.
/// A running agent (runtime_pid set) must appear in the cascade set so the
/// command can stop it before removing the record.
#[test]
fn cascade_includes_running_agent() {
    let agents = vec![make_agent("running-agent", Some(PERSONA_ID), Some(99999))];

    let pubkeys = collect_cascade_pubkeys(&agents, PERSONA_ID);

    assert_eq!(pubkeys, vec!["running-agent".to_string()]);
}

/// A failing agent-store save in Phase 3 must be retry-safe: the error
/// propagates before any keyring deletion or tombstone at the call site
/// (by construction — those side effects appear after the `?` in
/// `delete_persona`). Persona records and agent records are therefore
/// untouched on disk, so the command can be retried with no cleanup.
#[test]
fn failing_save_is_retry_safe() {
    let mut agents = vec![
        make_agent("pk-a", Some(PERSONA_ID), None),
        make_agent("pk-b", Some(PERSONA_ID), None),
        make_agent("pk-c", Some("custom:other"), None),
    ];
    let cascade: HashSet<String> = ["pk-a".to_string(), "pk-b".to_string()].into();

    let result = commit_cascade_agents(&mut agents, &cascade, |_| {
        Err("simulated disk failure".to_string())
    });

    assert!(
        result.is_err(),
        "commit must propagate the save error so callers can react"
    );
    // By construction: commit_cascade_agents returns Err before reaching the
    // keyring deletions and tombstones at the delete_persona call site.
    // Retrying delete_persona re-runs the full cascade cleanly from scratch.
}

/// A provider-deployed cascade target (non-local backend with a live
/// `backend_agent_id`) must be detected by the pre-flight so `delete_persona`
/// refuses the cascade before any destructive work. Local agents and
/// never-deployed provider agents must not block.
#[test]
fn remote_deployed_cascade_target_blocks_delete() {
    let agents = mixed_cascade_agents();
    let cascade: HashSet<String> = collect_cascade_pubkeys(&agents, PERSONA_ID)
        .into_iter()
        .collect();
    assert_eq!(cascade.len(), 3, "all three agents are cascade targets");

    let blockers = collect_remote_deployed(&agents, &cascade);

    assert_eq!(
        blockers,
        vec!["Deployed Agent".to_string()],
        "only the deployed provider agent blocks the cascade"
    );
}

/// A local agent, a deployed provider agent, and a provider agent that never
/// deployed — all linked to `PERSONA_ID`.
fn mixed_cascade_agents() -> Vec<ManagedAgentRecord> {
    let mut deployed = make_agent("pk-deployed", Some(PERSONA_ID), None);
    deployed.name = "Deployed Agent".to_string();
    deployed.backend = BackendKind::Provider {
        id: "blox".to_string(),
        config: serde_json::Value::Null,
    };
    deployed.backend_agent_id = Some("backend-1".to_string());

    // Provider backend but never deployed (no backend_agent_id) — not a blocker.
    let mut undeployed = make_agent("pk-undeployed", Some(PERSONA_ID), None);
    undeployed.backend = BackendKind::Provider {
        id: "blox".to_string(),
        config: serde_json::Value::Null,
    };

    vec![
        make_agent("pk-local", Some(PERSONA_ID), None),
        deployed,
        undeployed,
    ]
}

fn cascade_of(agents: &[ManagedAgentRecord]) -> HashSet<String> {
    collect_cascade_pubkeys(agents, PERSONA_ID)
        .into_iter()
        .collect()
}

/// Without the opt-in, a cascade containing a provider-deployed instance is
/// refused — the pre-existing contract, unchanged, so an IPC caller that never
/// learned about the flag sees exactly the error it saw before.
#[test]
fn preflight_refuses_remote_cascade_without_opt_in() {
    let agents = mixed_cascade_agents();
    let cascade = cascade_of(&agents);

    let result = preflight_remote_deployed_cascade(&agents, &cascade, PERSONA_ID, false);

    let error = result.expect_err("deployed provider instance must refuse the cascade");
    assert!(
        error.contains("Deployed Agent"),
        "error names the blocking instance: {error}"
    );
    assert!(
        error.contains("delete those agent instances first"),
        "error keeps its original wording: {error}"
    );
}

/// With the opt-in, the same cascade is allowed through. The caller has
/// acknowledged that the remote units keep running; the desktop cannot stop
/// them either way, so refusing only strands the records.
#[test]
fn preflight_allows_remote_cascade_with_opt_in() {
    let agents = mixed_cascade_agents();
    let cascade = cascade_of(&agents);

    assert!(
        preflight_remote_deployed_cascade(&agents, &cascade, PERSONA_ID, true).is_ok(),
        "opt-in admits the provider-backed cascade"
    );
}

/// A local-only cascade never consults the flag: it passes the pre-flight with
/// or without the opt-in, so ordinary persona deletes are untouched by this.
#[test]
fn preflight_ignores_opt_in_for_local_only_cascade() {
    let agents = vec![
        make_agent("pk-local-a", Some(PERSONA_ID), None),
        make_agent("pk-local-b", Some(PERSONA_ID), Some(4242)),
    ];
    let cascade = cascade_of(&agents);

    for force in [false, true] {
        assert!(
            preflight_remote_deployed_cascade(&agents, &cascade, PERSONA_ID, force).is_ok(),
            "local-only cascade passes with force_remote_delete={force}"
        );
    }
}

/// A provider agent that never completed a deploy has no remote unit to
/// orphan, so it must not require the opt-in.
#[test]
fn preflight_allows_never_deployed_provider_without_opt_in() {
    let mut undeployed = make_agent("pk-undeployed", Some(PERSONA_ID), None);
    undeployed.backend = BackendKind::Provider {
        id: "blox".to_string(),
        config: serde_json::Value::Null,
    };
    let agents = vec![undeployed];
    let cascade = cascade_of(&agents);

    assert!(
        preflight_remote_deployed_cascade(&agents, &cascade, PERSONA_ID, false).is_ok(),
        "never-deployed provider agent needs no acknowledgement"
    );
}
