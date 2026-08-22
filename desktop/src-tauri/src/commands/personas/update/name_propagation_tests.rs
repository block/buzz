//! Tests for `propagate_persona_name_rename` — the helper that propagates a
//! persona definition's display_name change to linked agent instances.

use super::*;

fn agent(persona_id: &str, name: &str, display_name: Option<&str>) -> ManagedAgentRecord {
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
        provider_policy_pending: false,
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
        respond_to: Default::default(),
        respond_to_allowlist: vec![],
        display_name: display_name.map(str::to_string),
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
        effort_level: None,
    }
}

#[test]
fn test_rename_propagates_to_matching_instance() {
    // An instance whose `name` equals the OLD persona display_name must get
    // both `name` and `display_name` updated to the new value.
    let mut records = vec![agent("persona-1", "Paul", Some("Paul"))];

    let renamed = propagate_persona_name_rename(&mut records, "persona-1", "Paul", "Paul Atreides");

    assert_eq!(
        renamed,
        vec!["pubkey-Paul".to_string()],
        "must report the renamed record's pubkey"
    );
    assert_eq!(records[0].name, "Paul Atreides", "name must be updated");
    assert_eq!(
        records[0].display_name,
        Some("Paul Atreides".to_string()),
        "display_name must be updated"
    );
    // The relay-profile sync params use `record.name`; after rename it carries
    // the new display_name, so the relay profile will be published with the correct name.
    assert_eq!(records[0].name, "Paul Atreides");
}

#[test]
fn test_rename_skips_pool_named_instance() {
    // A pool-named instance (e.g. "Birch") has a name DIFFERENT from the
    // persona display_name. It must keep its individualised name.
    let mut records = vec![agent("persona-1", "Birch", Some("Birch"))];

    let renamed = propagate_persona_name_rename(&mut records, "persona-1", "Paul", "Paul Atreides");

    assert!(
        renamed.is_empty(),
        "pool-named instance must not be reported as renamed"
    );
    assert_eq!(records[0].name, "Birch", "pool name must be preserved");
    assert_eq!(
        records[0].display_name,
        Some("Birch".to_string()),
        "pool display_name must be preserved"
    );
}

#[test]
fn test_rename_propagates_both_name_and_display_name() {
    // Explicit dual-field check: BOTH `name` and `display_name` must be
    // updated so the relay profile and the local UI are consistent.
    let mut records = vec![agent("persona-1", "OldName", None)];

    propagate_persona_name_rename(&mut records, "persona-1", "OldName", "NewName");

    assert_eq!(records[0].name, "NewName");
    assert_eq!(records[0].display_name, Some("NewName".to_string()));
}

#[test]
fn test_rename_only_affects_linked_persona() {
    // An instance linked to a DIFFERENT persona must not be touched, even
    // if it happens to carry the same display_name.
    let mut records = vec![
        agent("persona-1", "Paul", Some("Paul")),
        agent("persona-2", "Paul", Some("Paul")),
    ];

    propagate_persona_name_rename(&mut records, "persona-1", "Paul", "Paul Atreides");

    assert_eq!(records[0].name, "Paul Atreides", "linked instance renamed");
    assert_eq!(
        records[1].name, "Paul",
        "unrelated persona's instance untouched"
    );
}

#[test]
fn test_rename_renames_all_matching_instances_in_one_pass() {
    // Several instances may carry the definition name (multi-instance deploys
    // without a name pool): one call renames every match and reports each
    // pubkey, which is what the relay profile sync collection keys on.
    let mut records = vec![
        agent("persona-1", "Paul", Some("Paul")),
        agent("persona-1", "Paul", Some("Paul")),
        agent("persona-1", "Birch", Some("Birch")),
    ];
    records[1].pubkey = "pubkey-Paul-2".to_string();

    let renamed = propagate_persona_name_rename(&mut records, "persona-1", "Paul", "Duncan Idaho");

    assert_eq!(
        renamed,
        vec!["pubkey-Paul".to_string(), "pubkey-Paul-2".to_string()],
        "every matching instance's pubkey must be reported"
    );
    assert_eq!(records[0].name, "Duncan Idaho");
    assert_eq!(records[1].name, "Duncan Idaho");
    assert_eq!(records[2].name, "Birch", "pool-named instance untouched");
}

/// SAMI PROBE (fidelity pin for `sami_probe_rename_republishes_nonname_fields_from_stale_disk`
/// in `managed_agents/reconcile/tests.rs`): that probe hand-mutates `name` and
/// `display_name` to stand in for this helper. If the helper ever touched a
/// third field, the probe's fixture would silently stop modelling production.
/// Assert the mutation surface is EXACTLY those two fields, by diffing a
/// serialized before/after.
#[test]
fn rename_helper_mutates_only_name_and_display_name() {
    let mut records = vec![agent("persona-1", "Paul", Some("Paul"))];
    records[0].system_prompt = Some("disk prompt".into());
    records[0].parallelism = 7;
    let before = serde_json::to_value(&records[0]).unwrap();

    propagate_persona_name_rename(&mut records, "persona-1", "Paul", "Paul Atreides");

    let after = serde_json::to_value(&records[0]).unwrap();
    let changed: Vec<String> = before
        .as_object()
        .unwrap()
        .keys()
        .chain(after.as_object().unwrap().keys())
        .filter(|key| before.get(*key) != after.get(*key))
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    assert_eq!(
        changed,
        vec!["display_name".to_string(), "name".to_string()],
        "rename must mutate exactly name + display_name; a wider surface \
         invalidates the stale-disk republish probe's fixture"
    );
}

/// SAMI PROBE (hazard in the PROPOSED fix, not in the current code): the fix
/// for the stale-disk republish is "resolve the overlay before the write". At
/// this site the write is gated on `record.name == old_display_name`, and the
/// overlay REPLACES `record.name` with the relay's name. So resolving before
/// the gate can change which records the rename reaches.
///
/// Models a following device whose relay head carries a name that no longer
/// equals the persona's old display_name (device A already renamed, or the
/// instance is pool-named on the relay). Resolve-first makes the rename SKIP
/// the record entirely — the intended write is lost, which is the same
/// silent-data-loss class as the centralized-resolve probe.
#[test]
fn sami_probe_resolve_before_rename_can_skip_the_intended_rename() {
    // Disk name matches the old persona display_name, so production renames it.
    let mut disk_only = vec![agent("persona-1", "Paul", Some("Paul"))];
    let renamed =
        propagate_persona_name_rename(&mut disk_only, "persona-1", "Paul", "Paul Atreides");
    assert_eq!(
        renamed.len(),
        1,
        "control: against the DISK name the rename fires"
    );
    assert_eq!(disk_only[0].name, "Paul Atreides");

    // Same record after the overlay resolves a relay head whose name differs
    // (device A already applied the rename). `apply()` clobbers `record.name`.
    let mut overlay_resolved = vec![agent("persona-1", "Paul", Some("Paul"))];
    overlay_resolved[0].name = "Paul Atreides".to_string(); // what the overlay wrote

    let renamed_after_resolve =
        propagate_persona_name_rename(&mut overlay_resolved, "persona-1", "Paul", "Paul Atreides");

    assert!(
        renamed_after_resolve.is_empty(),
        "resolve-before-rename makes the gate miss: the record is NOT reported \
         as renamed, so update.rs never retains it and never syncs its relay \
         profile"
    );
    // Benign here (the names already agree), but the gate is now driven by
    // relay state rather than disk state — so the fix must resolve for the
    // PAYLOAD without moving the `name != old_display_name` decision onto the
    // resolved name.
}
