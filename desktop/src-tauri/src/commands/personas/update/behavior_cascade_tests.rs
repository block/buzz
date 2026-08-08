//! Tests for `propagate_persona_behavior` — the persona definition → linked
//! instance behavioral-group cascade added for issue #2501. The harness boots
//! `respond_to` from the instance record (`build_respond_to_env`), so a
//! persona edit that never reaches linked instances silently leaves every
//! running agent at its mint-time mode (owner-only by default).

use super::*;

use crate::managed_agents::RespondTo;

fn agent(persona_id: &str, name: &str, respond_to: RespondTo) -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: format!("pubkey-{name}"),
        name: name.to_string(),
        persona_id: Some(persona_id.to_string()),
        private_key_nsec: String::new(),
        auth_tag: None,
        relay_url: String::new(),
        avatar_url: None,
        acp_command: String::new(),
        agent_command: String::new(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 0,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        env_vars: std::collections::BTreeMap::new(),
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: Default::default(),
        backend_agent_id: None,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: String::new(),
        updated_at: String::new(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to,
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
        definition_respond_to: None,
        definition_respond_to_allowlist: vec![],
        definition_parallelism: None,
        relay_mesh: None,
    }
}

fn persona(
    respond_to: Option<&str>,
    respond_to_allowlist: Vec<String>,
    parallelism: Option<u32>,
) -> AgentDefinition {
    AgentDefinition {
        id: "persona-1".to_string(),
        display_name: "P".to_string(),
        avatar_url: None,
        system_prompt: String::new(),
        runtime: None,
        model: None,
        provider: None,
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        env_vars: std::collections::BTreeMap::new(),
        respond_to: respond_to.map(str::to_string),
        respond_to_allowlist,
        parallelism,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

/// The core #2501 contract: a persona behavior edit from 'owner-only' (the
/// default) to 'anyone' reaches a minted instance that never had an explicit
/// instance-level override.
#[test]
fn behavior_edit_cascades_to_inheriting_instance() {
    // Minted with no definition behavior → owner-only default (0.4.x era).
    let mut records = vec![agent("persona-1", "A1", RespondTo::OwnerOnly)];
    let updated = persona(Some("anyone"), vec![], Some(4));

    let touched = propagate_persona_behavior(
        &mut records,
        "persona-1",
        RespondTo::OwnerOnly,
        &updated,
    )
    .unwrap();

    assert!(touched);
    assert_eq!(records[0].respond_to, RespondTo::Anyone);
    assert!(records[0].respond_to_allowlist.is_empty());
    assert_eq!(records[0].parallelism, 4);
    // Mirror fields track the definition exactly.
    assert_eq!(records[0].definition_respond_to.as_deref(), Some("anyone"));
    assert!(records[0].definition_respond_to_allowlist.is_empty());
    assert_eq!(records[0].definition_parallelism, Some(4));
}

/// The instance-level explicit override is preserved: an instance whose
/// `respond_to` differs from the pre-edit definition value is a deliberate
/// pin, not inheritance, and must keep its own value (parity with the
/// pool-name rule in `propagate_persona_name_rename`).
#[test]
fn behavior_edit_preserves_instance_override() {
    let mut records = vec![agent("persona-1", "A1", RespondTo::Allowlist)];
    records[0].respond_to_allowlist = vec!["b".repeat(64)];
    let updated = persona(Some("anyone"), vec![], None);

    let touched = propagate_persona_behavior(
        &mut records,
        "persona-1",
        RespondTo::OwnerOnly, // pre-edit definition value
        &updated,
    )
    .unwrap();

    assert!(touched, "mirror refresh still counts as a touch");
    assert_eq!(
        records[0].respond_to,
        RespondTo::Allowlist,
        "explicit instance pin must survive"
    );
    assert_eq!(
        records[0].respond_to_allowlist,
        vec!["b".repeat(64)],
        "pinned allowlist must survive"
    );
    // Mirror still refreshes so mint/inspect paths see current definition bytes.
    assert_eq!(records[0].definition_respond_to.as_deref(), Some("anyone"));
}

/// Mixed fleets: adopting the new definition value happens per-instance, so an
/// inheriting instance updates while a pinned sibling keeps its override.
#[test]
fn behavior_edit_cascades_per_instance() {
    let mut records = vec![
        agent("persona-1", "Inherit", RespondTo::OwnerOnly),
        agent("persona-1", "Pinned", RespondTo::Allowlist),
        agent("persona-2", "Other", RespondTo::OwnerOnly),
    ];
    records[1].respond_to_allowlist = vec!["c".repeat(64)];
    let updated = persona(Some("allowlist"), vec!["d".repeat(64)], None);

    propagate_persona_behavior(
        &mut records,
        "persona-1",
        RespondTo::OwnerOnly,
        &updated,
    )
    .unwrap();

    assert_eq!(records[0].respond_to, RespondTo::Allowlist);
    assert_eq!(records[0].respond_to_allowlist, vec!["d".repeat(64)]);
    assert_eq!(records[1].respond_to, RespondTo::Allowlist);
    assert_eq!(
        records[1].respond_to_allowlist,
        vec!["c".repeat(64)],
        "pinned allowlist survives"
    );
    // Unrelated persona untouched.
    assert_eq!(records[2].respond_to, RespondTo::OwnerOnly);
    assert!(records[2].definition_respond_to.is_none());
}

/// Clearing a definition's behavioral group (edit to unset) drops inheriting
/// instances back to the harness default (owner-only), matching the
/// mint-time semantic of an undefined behavior group.
#[test]
fn behavior_clear_restores_default_on_inheriting_instance() {
    let mut records = vec![agent("persona-1", "A1", RespondTo::Anyone)];
    let cleared = persona(None, vec![], None);

    let touched = propagate_persona_behavior(
        &mut records,
        "persona-1",
        RespondTo::Anyone, // pre-edit definition value
        &cleared,
    )
    .unwrap();

    assert!(touched);
    assert_eq!(records[0].respond_to, RespondTo::OwnerOnly);
    assert!(records[0].respond_to_allowlist.is_empty());
    assert!(records[0].definition_respond_to.is_none());
}

/// An unknown-mode definition string fails loudly rather than silently
/// rewriting inheriting instances to a default the author did not choose —
/// the same fail-loudly contract `resolve_mint_behavioral_defaults` keeps at
/// mint time.
#[test]
fn unknown_definition_mode_fails_loudly() {
    let mut records = vec![agent("persona-1", "A1", RespondTo::OwnerOnly)];
    let mut bogus = persona(Some("anyone"), vec![], None);
    bogus.respond_to = Some("spaceship".to_string());

    let err = propagate_persona_behavior(
        &mut records,
        "persona-1",
        RespondTo::OwnerOnly,
        &bogus,
    )
    .unwrap_err();

    assert!(err.contains("spaceship"), "{err}");
    assert_eq!(
        records[0].respond_to,
        RespondTo::OwnerOnly,
        "failed cascade must not half-apply"
    );
}

/// `parallelism` cascades only when the definition carries an explicit value:
/// a definition without parallelism must not stomp an instance's configured
/// pool width (mint-time `None` → record keeps its own).
#[test]
fn absent_definition_parallelism_preserves_instance_value() {
    let mut records = vec![agent("persona-1", "A1", RespondTo::OwnerOnly)];
    records[0].parallelism = 8;
    let updated = persona(Some("anyone"), vec![], None);

    propagate_persona_behavior(
        &mut records,
        "persona-1",
        RespondTo::OwnerOnly,
        &updated,
    )
    .unwrap();

    assert_eq!(records[0].parallelism, 8);
    assert_eq!(records[0].definition_parallelism, None);
}
