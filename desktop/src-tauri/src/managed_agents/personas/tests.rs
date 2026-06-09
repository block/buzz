use super::{
    ensure_persona_ids_are_active, ensure_persona_is_active, merge_personas,
    migrate_retired_personas, validate_pack_id, validate_persona_activation_change,
    validate_persona_deletion, BUILT_IN_PERSONAS, RETIRED_PERSONAS,
};
use crate::managed_agents::PersonaRecord;

fn custom_persona(id: &str, display_name: &str) -> PersonaRecord {
    PersonaRecord {
        id: id.to_string(),
        display_name: display_name.to_string(),
        avatar_url: Some("https://example.com/avatar.png".to_string()),
        system_prompt: "Custom prompt".to_string(),
        runtime: None,
        model: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        source_pack: None,
        source_pack_persona_slug: None,
        env_vars: std::collections::BTreeMap::new(),
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
    assert!(records.iter().all(|record| record.is_active));
    let display_names: Vec<&str> = records
        .iter()
        .map(|record| record.display_name.as_str())
        .collect();
    assert_eq!(display_names, vec!["Solo", "Kit", "Scout"]);
}

#[test]
fn merge_personas_preserves_custom_records() {
    let custom = custom_persona("custom:test", "Custom");
    let (records, changed) = merge_personas(vec![custom.clone()], "2026-03-19T00:00:00Z");

    assert!(changed);
    assert!(records.iter().any(|record| record.id == custom.id));
}

#[test]
fn merge_personas_restores_builtin_defaults() {
    let mut edited_builtin = custom_persona("builtin:solo", "My Solo");
    edited_builtin.is_builtin = true;
    edited_builtin.is_active = true;
    let original_created_at = edited_builtin.created_at.clone();
    let original_updated_at = edited_builtin.updated_at.clone();

    let (records, changed) = merge_personas(vec![edited_builtin], "2026-03-19T00:00:00Z");

    assert!(changed);
    let solo = records
        .iter()
        .find(|record| record.id == "builtin:solo")
        .expect("solo built-in should exist");
    let canonical = BUILT_IN_PERSONAS
        .iter()
        .find(|persona| persona.id == "builtin:solo")
        .expect("solo built-in definition should exist");
    assert_eq!(solo.display_name, canonical.display_name);
    assert_eq!(solo.avatar_url.as_deref(), canonical.avatar_url,);
    assert_eq!(solo.created_at, original_created_at);
    assert_eq!(solo.updated_at, original_updated_at);
    assert!(solo.is_active);
}

#[test]
fn merge_personas_restores_builtin_env_vars() {
    // A hand-edited built-in record with stray env vars should be reset to
    // the canonical (empty) env on merge. Built-ins are intended immutable —
    // if a user wants per-persona credentials, they create or duplicate to a
    // custom persona.
    let mut tampered = custom_persona("builtin:solo", "Solo");
    tampered.is_builtin = true;
    tampered.avatar_url = None;
    tampered.is_active = true;
    tampered.env_vars =
        std::collections::BTreeMap::from([("ANTHROPIC_API_KEY".to_string(), "leaked".to_string())]);

    let (records, changed) = merge_personas(vec![tampered], "2026-03-19T00:00:00Z");

    assert!(changed);
    let solo = records
        .iter()
        .find(|record| record.id == "builtin:solo")
        .expect("solo built-in should exist");
    // Built-in persona definitions have no `env_vars` field — they are
    // always empty. The merge reset should clear the tampered key entirely.
    assert!(
        solo.env_vars.is_empty(),
        "expected empty, got {:?}",
        solo.env_vars
    );
}

#[test]
fn merge_personas_restores_builtin_name_pool_and_preserves_is_active() {
    let mut solo = custom_persona("builtin:solo", "Solo");
    solo.is_builtin = true;
    solo.avatar_url = None;
    solo.is_active = true;
    solo.name_pool = vec!["Definitely Not Solo".to_string()];

    let (records, changed) = merge_personas(vec![solo], "2026-03-19T00:00:00Z");

    assert!(changed);
    let solo = records
        .iter()
        .find(|record| record.id == "builtin:solo")
        .expect("solo built-in should exist");
    let expected_name_pool = BUILT_IN_PERSONAS
        .iter()
        .find(|persona| persona.id == "builtin:solo")
        .expect("solo built-in definition should exist")
        .name_pool
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    assert_eq!(solo.name_pool, expected_name_pool);
    assert!(solo.is_active);
}

#[test]
fn merge_personas_backfills_new_builtins_for_existing_store() {
    let mut legacy_builtins = vec![custom_persona("builtin:solo", "Solo")];
    for persona in &mut legacy_builtins {
        persona.is_builtin = true;
        persona.avatar_url = None;
    }

    let (records, changed) = merge_personas(legacy_builtins, "2026-03-19T00:00:00Z");

    assert!(changed);
    assert!(records.iter().any(|record| record.id == "builtin:kit"));
    assert!(records.iter().any(|record| record.id == "builtin:scout"));
    assert!(records.iter().any(|record| record.id == "builtin:solo"));
    assert!(
        records
            .iter()
            .find(|record| record.id == "builtin:solo")
            .expect("solo built-in should exist")
            .is_active
    );
    assert!(
        records
            .iter()
            .find(|record| record.id == "builtin:kit")
            .expect("kit built-in should exist")
            .is_active
    );
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

    assert_eq!(err, "persona missing not found");
}

#[test]
fn ensure_persona_is_active_rejects_inactive_personas() {
    let mut persona = custom_persona("builtin:solo", "Solo");
    persona.is_builtin = true;
    persona.is_active = false;

    let err = ensure_persona_is_active(&[persona], "builtin:solo").unwrap_err();

    assert_eq!(
        err,
        "Solo is not in My Agents. Choose it from Persona Catalog first."
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
        "Only built-in personas can be added to or removed from My Agents."
    );
}

#[test]
fn validate_persona_activation_change_rejects_managed_agent_references() {
    let mut persona = custom_persona("builtin:solo", "Solo");
    persona.is_builtin = true;

    let err = validate_persona_activation_change(&persona, false, true, false).unwrap_err();

    assert_eq!(
        err,
        "Solo is still assigned to a managed agent. Remove or reassign those agents first."
    );
}

#[test]
fn validate_persona_activation_change_rejects_team_references() {
    let mut persona = custom_persona("builtin:solo", "Solo");
    persona.is_builtin = true;

    let err = validate_persona_activation_change(&persona, false, false, true).unwrap_err();

    assert_eq!(
        err,
        "Solo is still referenced by a team. Remove it from those teams first."
    );
}

#[test]
fn validate_persona_activation_change_allows_safe_builtin_updates() {
    let mut persona = custom_persona("builtin:solo", "Solo");
    persona.is_builtin = true;

    assert!(validate_persona_activation_change(&persona, true, false, false).is_ok());
    assert!(validate_persona_activation_change(&persona, false, false, false).is_ok());
}

#[test]
fn validate_persona_deletion_rejects_builtins() {
    let mut persona = custom_persona("builtin:solo", "Solo");
    persona.is_builtin = true;

    let err = validate_persona_deletion(&persona, false).unwrap_err();

    assert_eq!(err, "Built-in personas cannot be deleted.");
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

// ── validate_pack_id ──────────────────────────────────────────────────────────

#[test]
fn pack_id_valid_reverse_dns() {
    assert!(validate_pack_id("com.example.security-team").is_ok());
}

#[test]
fn pack_id_valid_simple() {
    assert!(validate_pack_id("my-pack").is_ok());
}

#[test]
fn pack_id_rejects_empty() {
    assert!(validate_pack_id("").is_err());
}

#[test]
fn pack_id_rejects_dot_dot_path_traversal() {
    // Critical regression test: ".." must never pass validation.
    // A pack with id ".." would write into the parent directory.
    assert!(validate_pack_id("..").is_err());
}

#[test]
fn pack_id_rejects_single_dot() {
    assert!(validate_pack_id(".").is_err());
}

#[test]
fn pack_id_rejects_leading_dot() {
    assert!(validate_pack_id(".hidden").is_err());
}

#[test]
fn pack_id_rejects_slashes() {
    assert!(validate_pack_id("../etc/passwd").is_err());
    assert!(validate_pack_id("foo/bar").is_err());
}

#[test]
fn pack_id_rejects_no_alphanumeric() {
    assert!(validate_pack_id("---").is_err());
    assert!(validate_pack_id("___").is_err());
}

#[test]
fn pack_id_rejects_too_long() {
    let long_id = "a".repeat(129);
    assert!(validate_pack_id(&long_id).is_err());
    // 128 chars is fine
    let max_id = "a".repeat(128);
    assert!(validate_pack_id(&max_id).is_ok());
}

// ── migrate_retired_personas ──────────────────────────────────────────────────

#[test]
fn migrate_retires_unmodified_personas() {
    let now = "2026-04-01T00:00:00Z";
    // Simulate a store from before the Solo/Kit/Scout transition: all 6
    // retired personas with original system prompts.
    let mut stored: Vec<PersonaRecord> = RETIRED_PERSONAS
        .iter()
        .map(|(id, prompt)| PersonaRecord {
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
    let mut stored = vec![PersonaRecord {
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
    let mut stored_with_retired = vec![PersonaRecord {
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
    let mut stored_pre_demotion = vec![PersonaRecord {
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
