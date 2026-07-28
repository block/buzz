use super::{
    built_in_persona_records, ensure_persona_ids_are_active, ensure_persona_is_active,
    merge_personas, migrate_retired_personas, validate_persona_activation_change,
    validate_persona_deletion, BUILT_IN_PERSONAS, RETIRED_PERSONAS,
};
use crate::managed_agents::discovery::{default_agent_command, effective_agent_command};
use crate::managed_agents::AgentDefinition;

const COMMAND_TEAM_PERSONAS: [(&str, &str, &str); 8] = [
    (
        "builtin:command-chief-of-staff",
        "Chief of Staff",
        "challenge inconsistencies",
    ),
    (
        "builtin:command-operations",
        "Operations Adviser",
        "readiness, dependencies, risks",
    ),
    (
        "builtin:command-intelligence",
        "Maritime N2 Adviser",
        "reported information",
    ),
    (
        "builtin:command-logistics",
        "Logistics Adviser",
        "replenishment",
    ),
    (
        "builtin:command-navigation",
        "Navigation Adviser",
        "navigation evidence",
    ),
    (
        "builtin:command-daily-routine",
        "Daily Routine Adviser",
        "calendar, reminders, deadlines",
    ),
    (
        "builtin:command-reporting",
        "Reporting Adviser",
        "reports, returns, missing inputs",
    ),
    (
        "builtin:command-plans",
        "Plans Adviser",
        "medium- and long-range",
    ),
];

fn custom_persona(id: &str, display_name: &str) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        display_name: display_name.to_string(),
        avatar_url: Some("https://example.com/avatar.png".to_string()),
        system_prompt: "Custom prompt".to_string(),
        runtime: None,
        model: None,
        provider: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        env_vars: std::collections::BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2026-03-19T00:00:00Z".to_string(),
        updated_at: "2026-03-19T00:00:00Z".to_string(),
    }
}

#[test]
fn merge_personas_adds_missing_built_ins() {
    let (records, changed) = merge_personas(Vec::new(), "2026-03-19T00:00:00Z");

    assert!(changed);
    assert_eq!(records.len(), BUILT_IN_PERSONAS.len());
    assert!(records.iter().all(|record| record.is_builtin));
    assert!(records
        .iter()
        .any(|record| record.id == "builtin:fizz" && record.runtime.is_none()));
    let display_names: Vec<&str> = records
        .iter()
        .map(|record| record.display_name.as_str())
        .collect();
    assert_eq!(
        display_names,
        vec![
            "Fizz",
            "Honey",
            "Bumble",
            "Chief of Staff",
            "Operations Adviser",
            "Maritime N2 Adviser",
            "Logistics Adviser",
            "Navigation Adviser",
            "Daily Routine Adviser",
            "Reporting Adviser",
            "Plans Adviser",
        ]
    );
    let active_ids: Vec<&str> = records
        .iter()
        .filter(|record| record.is_active)
        .map(|record| record.id.as_str())
        .collect();
    assert_eq!(
        active_ids,
        vec![
            "builtin:fizz",
            "builtin:honey",
            "builtin:bumble",
            "builtin:command-chief-of-staff",
            "builtin:command-operations",
            "builtin:command-intelligence",
            "builtin:command-logistics",
            "builtin:command-navigation",
            "builtin:command-daily-routine",
            "builtin:command-reporting",
            "builtin:command-plans",
        ]
    );
}

#[test]
fn command_team_builtins_are_active_symbolic_and_route_independent() {
    let records = built_in_persona_records("2026-07-27T00:00:00Z");

    for (id, display_name, role_fragment) in COMMAND_TEAM_PERSONAS {
        let matches = records
            .iter()
            .filter(|record| record.id == id)
            .collect::<Vec<_>>();
        assert_eq!(matches.len(), 1, "{id} should appear exactly once");
        let record = matches[0];
        assert_eq!(record.display_name, display_name);
        assert!(record.is_builtin);
        assert!(record.is_active);
        assert_eq!(record.runtime, None);
        assert_eq!(record.model, None);
        assert!(
            record
                .avatar_url
                .as_deref()
                .is_some_and(|avatar| avatar.starts_with("data:image/svg+xml,")),
            "{id} should use a compact symbolic SVG avatar"
        );
        assert!(
            record.system_prompt.contains(role_fragment),
            "{id} should retain its adviser role boundary"
        );
        assert!(record
            .system_prompt
            .contains("command-discussion-outcome-v1"));
        assert!(record
            .system_prompt
            .contains("mem/command-brief/<adviser>/<yyyy-mm-dd>/<outcome-id>"));
        assert!(record.system_prompt.contains("Recorded for future briefs"));
        assert!(record
            .system_prompt
            .contains("only after `buzz mem set` succeeds"));
        assert!(record
            .system_prompt
            .contains("Do not copy the raw Buzz transcript"));
        assert!(record
            .system_prompt
            .contains("<persona-id>\\n<channel-id>\\n<triggering-event-id>"));
        assert!(record.system_prompt.contains(
            "Seek applicable doctrine with search_command_doctrine before substantive advice."
        ));
        assert!(record.system_prompt.contains(
            "If no applicable doctrine is retrieved, continue with a reasoned assessment."
        ));
    }
}

#[test]
fn n2_and_planning_prompts_pin_osint_provenance_and_virtual_jpg_behaviour() {
    let records = built_in_persona_records("2026-07-28T00:00:00Z");
    let n2 = records
        .iter()
        .find(|record| record.id == "builtin:command-intelligence")
        .expect("N2 adviser should exist");
    for required in [
        "world_monitor_",
        "MUST call at least one relevant world_monitor_ tool",
        "Do not send a pickup acknowledgement",
        "same turn",
        "reported information",
        "observed indicators",
        "assumptions",
        "assessment",
        "ISO 3166-1 alpha-2",
    ] {
        assert!(n2.system_prompt.contains(required), "missing {required:?}");
    }

    for persona_id in [
        "builtin:command-chief-of-staff",
        "builtin:command-operations",
    ] {
        let prompt = &records
            .iter()
            .find(|record| record.id == persona_id)
            .expect("planning persona should exist")
            .system_prompt;
        for required in [
            "mission-specific Buzz channel",
            "virtual Joint Planning Group",
            "@mention",
        ] {
            assert!(
                prompt.contains(required),
                "{persona_id} missing {required:?}"
            );
        }
    }

    let (twice, changed) = merge_personas(records.clone(), "2026-07-28T00:00:01Z");
    assert!(!changed);
    for persona_id in ["builtin:command-intelligence", "builtin:command-logistics"] {
        assert_eq!(
            twice
                .iter()
                .filter(|record| record.id == persona_id)
                .count(),
            1
        );
    }
}

#[test]
fn merge_personas_upgrades_the_unmodified_n2_prompt_without_overwriting_custom_edits() {
    let current = built_in_persona_records("2026-07-28T00:00:00Z")
        .into_iter()
        .find(|record| record.id == "builtin:command-intelligence")
        .expect("N2 adviser should exist");

    let mut previous = current.clone();
    previous.system_prompt = super::PREVIOUS_COMMAND_INTELLIGENCE_PROMPT.to_string();
    let (upgraded, changed) = merge_personas(vec![previous], "2026-07-28T00:00:01Z");
    assert!(changed);
    let upgraded_n2 = upgraded
        .iter()
        .find(|record| record.id == "builtin:command-intelligence")
        .expect("upgraded N2 adviser should exist");
    assert_eq!(upgraded_n2.system_prompt, current.system_prompt);
    assert_eq!(upgraded_n2.updated_at, "2026-07-28T00:00:01Z");

    let mut customised = current.clone();
    customised.system_prompt.push_str("\nUser customisation.");
    let (preserved, changed) = merge_personas(vec![customised.clone()], "2026-07-28T00:00:02Z");
    assert!(
        changed,
        "the other missing built-ins should still be inserted"
    );
    let preserved_n2 = preserved
        .iter()
        .find(|record| record.id == "builtin:command-intelligence")
        .expect("customised N2 adviser should exist");
    assert_eq!(preserved_n2.system_prompt, customised.system_prompt);
}

#[test]
fn navigation_builtin_remains_advisory() {
    let records = built_in_persona_records("2026-07-27T00:00:00Z");
    let navigation = records
        .iter()
        .find(|record| record.id == "builtin:command-navigation")
        .expect("navigation adviser should exist");

    assert!(navigation
        .system_prompt
        .contains("do not make navigational decisions"));
    assert!(navigation
        .system_prompt
        .contains("do not generate executable navigation orders"));
}

#[test]
fn merge_personas_preserves_custom_records() {
    let custom = custom_persona("custom:test", "Custom");
    let (records, changed) = merge_personas(vec![custom.clone()], "2026-03-19T00:00:00Z");

    assert!(changed);
    assert!(records.iter().any(|record| record.id == custom.id));
}

#[test]
fn merge_personas_preserves_builtin_edits() {
    let mut edited_builtin = custom_persona("builtin:fizz", "My Fizz");
    edited_builtin.is_builtin = true;
    edited_builtin.is_active = true;
    edited_builtin.system_prompt = "User-edited instructions".to_string();
    edited_builtin.name_pool = vec!["User-edited name".to_string()];
    edited_builtin.env_vars =
        std::collections::BTreeMap::from([("USER_SETTING".to_string(), "value".to_string())]);

    let (records, changed) = merge_personas(vec![edited_builtin.clone()], "2026-03-19T00:00:00Z");

    assert!(changed); // The remaining seeded built-ins are added.
    let fizz = records
        .iter()
        .find(|record| record.id == "builtin:fizz")
        .expect("fizz built-in should exist");
    assert_eq!(fizz.display_name, edited_builtin.display_name);
    assert_eq!(fizz.system_prompt, edited_builtin.system_prompt);
    assert_eq!(fizz.name_pool, edited_builtin.name_pool);
    assert_eq!(fizz.env_vars, edited_builtin.env_vars);
    assert_eq!(fizz.is_active, edited_builtin.is_active);
}

#[test]
fn merge_personas_restores_builtin_marker_without_resetting_edits() {
    let mut edited_builtin = custom_persona("builtin:fizz", "My Fizz");
    edited_builtin.is_builtin = false;

    let (records, changed) = merge_personas(vec![edited_builtin], "2026-03-19T00:00:00Z");

    assert!(changed);
    let fizz = records
        .iter()
        .find(|record| record.id == "builtin:fizz")
        .expect("fizz built-in should exist");
    assert!(fizz.is_builtin);
    assert_eq!(fizz.display_name, "My Fizz");
}

#[test]
fn merge_personas_adds_fizz_and_retires_old_builtins_for_existing_store() {
    let mut legacy_builtins = vec![custom_persona("builtin:solo", "Solo")];
    for persona in &mut legacy_builtins {
        persona.is_builtin = true;
        persona.avatar_url = None;
    }

    let (records, changed) = merge_personas(legacy_builtins, "2026-03-19T00:00:00Z");

    assert!(changed);
    let fizz = records
        .iter()
        .find(|record| record.id == "builtin:fizz")
        .expect("fizz built-in should exist");
    assert!(fizz.is_builtin);
    assert!(fizz.is_active);

    let solo = records
        .iter()
        .find(|record| record.id == "builtin:solo")
        .expect("old solo record should be retained as retired custom persona");
    assert!(!solo.is_builtin);
    assert!(!solo.is_active);
    assert_eq!(solo.display_name, "Solo (retired)");
}

#[test]
fn merge_personas_demotes_retired_builtins() {
    // custom_persona uses "Custom prompt", which doesn't match the original
    // retired system prompt, so the migration pass soft-deprecates rather
    // than removes the record.
    let mut retired = custom_persona("builtin:reviewer", "Reviewer");
    retired.is_builtin = true;
    retired.is_active = true;
    let original_created_at = retired.created_at.clone();

    let (records, changed) = merge_personas(vec![retired], "2026-04-01T00:00:00Z");

    assert!(changed);
    let demoted = records
        .iter()
        .find(|record| record.id == "builtin:reviewer")
        .expect("retired built-in should be retained as a soft-deprecated custom persona");
    assert!(!demoted.is_builtin);
    // migrate_retired_personas deactivates customized retired personas.
    assert!(!demoted.is_active);
    assert_eq!(demoted.display_name, "Reviewer (retired)");
    assert_eq!(demoted.created_at, original_created_at);
    assert_eq!(demoted.updated_at, "2026-04-01T00:00:00Z");
}

#[test]
fn ensure_persona_is_active_rejects_missing_personas() {
    let err = ensure_persona_is_active(&[], "missing").unwrap_err();

    assert_eq!(err, "agent missing not found");
}

#[test]
fn ensure_persona_is_active_rejects_inactive_personas() {
    let mut persona = custom_persona("builtin:fizz", "Fizz");
    persona.is_builtin = true;
    persona.is_active = false;

    let err = ensure_persona_is_active(&[persona], "builtin:fizz").unwrap_err();

    assert_eq!(
        err,
        "Fizz is not in My Agents. Choose it from Agent Catalog first."
    );
}

#[test]
fn ensure_persona_ids_are_active_checks_each_requested_id() {
    let personas = vec![
        custom_persona("custom:alpha", "Alpha"),
        custom_persona("custom:beta", "Beta"),
    ];

    assert!(ensure_persona_ids_are_active(
        &personas,
        &["custom:alpha".to_string(), "custom:beta".to_string()],
    )
    .is_ok());
}

#[test]
fn validate_persona_activation_change_rejects_non_builtins() {
    let persona = custom_persona("custom:alpha", "Alpha");

    let err = validate_persona_activation_change(&persona, false, false, false).unwrap_err();

    assert_eq!(
        err,
        "Only built-in agents can be added to or removed from My Agents."
    );
}

#[test]
fn validate_persona_activation_change_rejects_managed_agent_references() {
    let mut persona = custom_persona("builtin:fizz", "Fizz");
    persona.is_builtin = true;

    let err = validate_persona_activation_change(&persona, false, true, false).unwrap_err();

    assert_eq!(
        err,
        "Fizz is still assigned to a managed agent. Remove or reassign those agents first."
    );
}

#[test]
fn validate_persona_activation_change_rejects_team_references() {
    let mut persona = custom_persona("builtin:fizz", "Fizz");
    persona.is_builtin = true;

    let err = validate_persona_activation_change(&persona, false, false, true).unwrap_err();

    assert_eq!(
        err,
        "Fizz is still referenced by a team. Remove it from those teams first."
    );
}

#[test]
fn validate_persona_activation_change_allows_safe_builtin_updates() {
    let mut persona = custom_persona("builtin:fizz", "Fizz");
    persona.is_builtin = true;

    assert!(validate_persona_activation_change(&persona, true, false, false).is_ok());
    assert!(validate_persona_activation_change(&persona, false, false, false).is_ok());
}

#[test]
fn validate_persona_deletion_rejects_builtins() {
    let mut persona = custom_persona("builtin:fizz", "Fizz");
    persona.is_builtin = true;

    let err = validate_persona_deletion(&persona, false).unwrap_err();

    assert_eq!(err, "Built-in agents cannot be deleted.");
}

#[test]
fn validate_persona_deletion_rejects_team_references() {
    let persona = custom_persona("custom:alpha", "Alpha");

    let err = validate_persona_deletion(&persona, true).unwrap_err();

    assert_eq!(
        err,
        "Alpha is still referenced by a team. Remove it from those teams first."
    );
}

#[test]
fn validate_persona_deletion_allows_safe_custom_personas() {
    let persona = custom_persona("custom:alpha", "Alpha");

    assert!(validate_persona_deletion(&persona, false).is_ok());
}

// ── migrate_retired_personas ──────────────────────────────────────────────────

#[test]
fn migrate_retires_unmodified_personas() {
    let now = "2026-04-01T00:00:00Z";
    // Simulate a store from before the Fizz transition: all 6
    // retired personas with original system prompts.
    let mut stored: Vec<AgentDefinition> = RETIRED_PERSONAS
        .iter()
        .map(|(id, prompt)| AgentDefinition {
            id: id.to_string(),
            system_prompt: prompt.to_string(),
            is_builtin: false, // already demoted by merge_personas
            ..custom_persona(id, "Test Persona")
        })
        .collect();

    let changed = migrate_retired_personas(&mut stored, now);

    assert!(changed);
    assert_eq!(
        stored.len(),
        RETIRED_PERSONAS.len(),
        "all retired personas should be soft-deprecated, not removed",
    );
    assert!(
        stored
            .iter()
            .all(|r| r.display_name.ends_with(" (retired)")),
        "all retired personas should have ' (retired)' suffix",
    );
    assert!(
        stored.iter().all(|r| !r.is_active),
        "all retired personas should be inactive",
    );
    assert!(
        stored.iter().all(|r| r.updated_at == now),
        "all retired personas should have refreshed updated_at",
    );
}

#[test]
fn migrate_preserves_customized_personas() {
    let now = "2026-04-01T00:00:00Z";
    let mut stored = vec![AgentDefinition {
        id: "builtin:researcher".to_string(),
        display_name: "My Researcher".to_string(),
        system_prompt: "My custom research workflow with special instructions".to_string(),
        is_builtin: false,
        is_active: true,
        ..custom_persona("builtin:researcher", "My Researcher")
    }];

    let changed = migrate_retired_personas(&mut stored, now);

    assert!(changed);
    assert_eq!(stored.len(), 1);
    let record = &stored[0];
    assert_eq!(record.display_name, "My Researcher (retired)");
    assert!(!record.is_active);
    assert_eq!(
        record.system_prompt,
        "My custom research workflow with special instructions"
    );
    assert_eq!(record.updated_at, now);
}

#[test]
fn migrate_is_idempotent() {
    let now = "2026-04-01T00:00:00Z";

    // 1. Non-retired persona — no-op.
    let mut stored = vec![custom_persona("custom:test", "Custom")];
    assert!(!migrate_retired_personas(&mut stored, now));
    assert_eq!(stored.len(), 1);

    // 2. Already-retired persona (display_name ends with " (retired)") — no-op.
    let mut stored_with_retired = vec![AgentDefinition {
        id: "builtin:researcher".to_string(),
        display_name: "Researcher (retired)".to_string(),
        system_prompt: "My custom prompt".to_string(),
        is_builtin: false,
        is_active: false,
        ..custom_persona("builtin:researcher", "Researcher (retired)")
    }];
    assert!(
        !migrate_retired_personas(&mut stored_with_retired, now),
        "already-retired persona should not trigger another change"
    );

    // 3. Retired persona still marked is_builtin: true (pre-demotion).
    // migrate_retired_personas should still soft-deprecate it.
    let mut stored_pre_demotion = vec![AgentDefinition {
        id: "builtin:reviewer".to_string(),
        display_name: "Reviewer".to_string(),
        system_prompt: "Custom review prompt".to_string(),
        is_builtin: true,
        is_active: true,
        ..custom_persona("builtin:reviewer", "Reviewer")
    }];
    assert!(migrate_retired_personas(&mut stored_pre_demotion, now));
    assert_eq!(stored_pre_demotion[0].display_name, "Reviewer (retired)");
    assert!(!stored_pre_demotion[0].is_active);

    // 4. Run again on result of (3) — should be no-op.
    assert!(!migrate_retired_personas(&mut stored_pre_demotion, now));
}

// ── Fizz default harness ──────────────────────────────────────────────────────

#[test]
fn fizz_builtin_has_no_pinned_runtime() {
    // The Fizz built-in must not hard-pin a runtime so it inherits the
    // bundled default (buzz-agent) rather than requiring goose on PATH.
    let records = built_in_persona_records("2026-01-01T00:00:00Z");
    let fizz = records
        .iter()
        .find(|r| r.id == "builtin:fizz")
        .expect("builtin:fizz must exist");
    assert_eq!(
        fizz.runtime, None,
        "Fizz built-in must not pin a runtime — it should inherit the default"
    );
}

#[test]
fn fizz_builtin_resolves_to_buzz_agent() {
    // With no runtime pin, effective_agent_command must fall through to
    // default_agent_command(), which resolves the bundled buzz-agent.
    let records = built_in_persona_records("2026-01-01T00:00:00Z");
    assert_eq!(
        effective_agent_command(Some("builtin:fizz"), &records, None),
        default_agent_command(),
        "Fizz must resolve to the bundled default harness, not goose"
    );
    assert_eq!(
        effective_agent_command(Some("builtin:fizz"), &records, None),
        "buzz-agent",
        "Fizz must resolve to buzz-agent specifically"
    );
}

#[test]
fn command_adviser_builtins_default_to_single_turn_parallelism() {
    let records = built_in_persona_records("2026-07-28T00:00:00Z");
    let command_team: Vec<_> = records
        .iter()
        .filter(|persona| persona.id.starts_with("builtin:command-"))
        .collect();

    assert!(!command_team.is_empty(), "Command Team personas must exist");
    assert!(
        command_team
            .iter()
            .all(|persona| persona.parallelism == Some(1)),
        "every Command Team persona must default to one concurrent turn"
    );

    let fizz = records
        .iter()
        .find(|persona| persona.id == "builtin:fizz")
        .expect("Fizz built-in must exist");
    assert_eq!(
        fizz.parallelism, None,
        "the Command Team limit must not change unrelated built-ins"
    );
}
