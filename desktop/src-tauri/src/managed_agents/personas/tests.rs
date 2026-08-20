use super::{
    built_in_persona_records, ensure_persona_ids_are_active, ensure_persona_is_active,
    merge_personas, migrate_retired_personas, validate_persona_activation_change,
    validate_persona_deletion, BUILT_IN_PERSONAS, RETIRED_PERSONAS,
};
use crate::managed_agents::discovery::{default_agent_command, effective_agent_command};
use crate::managed_agents::AgentDefinition;

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
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
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
    assert_eq!(display_names, vec!["Fizz", "Honey", "Pollen"]);
    let active_ids: Vec<&str> = records
        .iter()
        .filter(|record| record.is_active)
        .map(|record| record.id.as_str())
        .collect();
    assert_eq!(
        active_ids,
        vec!["builtin:fizz", "builtin:honey", "builtin:bumble"]
    );
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

    assert_eq!(err, "Fizz is not in My Agents.");
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
        shared: false,
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
        shared: false,
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
        shared: false,
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

// ── §2.7 merge-preserving persona save (P3-C1 / P4-C1 acceptance) ─────────────

use super::{
    load_persona_views_at, load_personas_at, merge_preserving_definitions, save_personas_at,
};
use crate::managed_agents::storage::{load_agent_definitions_at, save_agent_definitions_at};
use crate::managed_agents::ManagedAgentRecord;

/// A keyless definition projected from a shared library entry: it carries
/// `library_ref`/`library_applied_revision`, which live ONLY on
/// `ManagedAgentRecord` and are exactly what head's wholesale save erased.
fn projected_record(slug: &str, revision: u64) -> ManagedAgentRecord {
    let mut record = custom_persona(slug, "Shared Agent").into_agent_record();
    record.library_ref = Some(format!("lib-{slug}"));
    record.library_applied_revision = Some(revision);
    record
}

/// Assert the projected record survives a save with its library metadata and
/// index invariant intact. `label` names the writer shape for failure output.
fn assert_projection_survived(saved: &[ManagedAgentRecord], label: &str) {
    let projected = saved
        .iter()
        .find(|record| record.slug.as_deref() == Some("shared"))
        .unwrap_or_else(|| panic!("{label}: projected record must survive the save"));
    assert_eq!(
        projected.library_ref.as_deref(),
        Some("lib-shared"),
        "{label}: library_ref must survive an unrelated writer",
    );
    assert_eq!(
        projected.library_applied_revision,
        Some(3),
        "{label}: library_applied_revision must survive an unrelated writer",
    );
    assert_eq!(
        saved
            .iter()
            .filter(|record| record.library_ref.as_deref() == Some("lib-shared"))
            .count(),
        1,
        "{label}: exactly one keyless record may link the library entry",
    );
}

/// P3-C1 acceptance: every persona writer funnels through `save_personas[_at]`,
/// so a save that touches ONLY an unrelated persona must leave a projected
/// record's library metadata untouched. At head, `into_agent_record`
/// reconstructed the whole vector and erased it. Each closure models one
/// writer's vector transformation (create, update, activation toggle, delete,
/// inbound upsert, inbound tombstone, team import, team delete) applied to the
/// unrelated persona — never the projected one.
#[test]
fn merge_preserving_save_keeps_library_metadata_across_every_writer_shape() {
    let seed = || {
        vec![
            projected_record("shared", 3),
            custom_persona("custom:plain", "Plain").into_agent_record(),
        ]
    };

    type Shape = (
        &'static str,
        fn(Vec<AgentDefinition>) -> Vec<AgentDefinition>,
    );
    let shapes: Vec<Shape> = vec![
        ("create", |mut views| {
            views.push(custom_persona("custom:new", "New"));
            views
        }),
        ("update", |mut views| {
            for view in &mut views {
                if view.id == "custom:plain" {
                    view.display_name = "Renamed".to_string();
                }
            }
            views
        }),
        ("activation_toggle", |mut views| {
            for view in &mut views {
                if view.id == "custom:plain" {
                    view.is_active = !view.is_active;
                }
            }
            views
        }),
        ("delete_unrelated", |views| {
            views
                .into_iter()
                .filter(|v| v.id != "custom:plain")
                .collect()
        }),
        ("inbound_upsert_unrelated", |mut views| {
            views.push(custom_persona("custom:inbound", "Inbound"));
            views
        }),
        ("inbound_tombstone_unrelated", |views| {
            views
                .into_iter()
                .filter(|v| v.id != "custom:plain")
                .collect()
        }),
        ("team_import", |mut views| {
            let mut member = custom_persona("team:member", "Team Member");
            member.source_team = Some("team-1".to_string());
            views.push(member);
            views
        }),
        ("team_delete", |views| {
            views
                .into_iter()
                .filter(|v| !v.id.starts_with("team:"))
                .collect()
        }),
    ];

    for (label, transform) in shapes {
        let existing = seed();
        // A writer always passes the definition VIEW of the current store —
        // library metadata is stripped because `AgentDefinition` cannot carry
        // it. The seam must re-merge it from the canonical raw record.
        let views: Vec<AgentDefinition> = existing
            .iter()
            .filter_map(|record| record.to_definition_view())
            .collect();
        let saved = merge_preserving_definitions(existing, &transform(views))
            .unwrap_or_else(|e| panic!("{label}: unrelated writer must not fail: {e}"));
        assert_projection_survived(&saved, label);
    }
}

/// §2.7 fail-closed routing (P4-C1/P4-C2): a plain persona save must not be
/// able to DELETE a library-projected record. Dropping the projected view from
/// the save vector is a removal that must advance the §3.4 `ExcludePending`
/// state machine; until that lands the seam rejects it rather than silently
/// dropping the row.
#[test]
fn merge_preserving_save_rejects_deleting_a_projected_record() {
    let existing = vec![
        projected_record("shared", 3),
        custom_persona("custom:plain", "Plain").into_agent_record(),
    ];
    // The save vector omits "shared" entirely — a deletion of the projection.
    let views = vec![custom_persona("custom:plain", "Plain")];
    let err = merge_preserving_definitions(existing, &views)
        .expect_err("deleting a projected record must fail closed");
    assert!(err.contains("shared") && err.contains("removal"), "{err}");
}

/// §2.7 fail-closed routing (P4-C2): a plain persona save must not overwrite a
/// projected record's shared content. A view that changes a shared slot is a
/// library edit / library-linked inbound upsert that must route through the
/// library with a revision, not become an unjournaled local edit.
#[test]
fn merge_preserving_save_rejects_editing_a_projected_records_shared_slot() {
    let existing = vec![projected_record("shared", 3)];
    // Re-pass the projected view but mutate a shared slot (display name).
    let mut view = existing[0].to_definition_view().expect("view");
    view.display_name = "Hijacked".to_string();
    let err = merge_preserving_definitions(existing, &[view])
        .expect_err("editing projected shared content must fail closed");
    assert!(err.contains("shared") && err.contains("edits"), "{err}");
}

/// The no-op case that keeps the every-writer guarantee honest: re-passing a
/// projected record's own view (metadata stripped, no field changed) is NOT a
/// mutation and must ride through intact — otherwise an unrelated writer that
/// re-serializes the whole store would trip the fail-closed guard.
#[test]
fn merge_preserving_save_passes_unchanged_projected_record_through() {
    let existing = vec![
        projected_record("shared", 3),
        custom_persona("custom:plain", "Plain").into_agent_record(),
    ];
    let views: Vec<AgentDefinition> = existing
        .iter()
        .filter_map(|record| record.to_definition_view())
        .collect();
    let saved =
        merge_preserving_definitions(existing, &views).expect("unchanged projection must pass");
    assert_projection_survived(&saved, "noop_reserialize");
}

/// `MutationRoute::for_slug` classifies a projected slug as `LibraryProjected`,
/// a plain slug and an unknown slug (a create) as `Plain` — the decision every
/// §3 delete/inbound/import caller consults before the plain path.
#[test]
fn mutation_route_classifies_projected_plain_and_unknown_slugs() {
    let records = vec![
        projected_record("shared", 3),
        custom_persona("custom:plain", "Plain").into_agent_record(),
    ];
    assert_eq!(
        super::MutationRoute::for_slug(&records, "shared"),
        super::MutationRoute::LibraryProjected,
    );
    assert_eq!(
        super::MutationRoute::for_slug(&records, "custom:plain"),
        super::MutationRoute::Plain,
    );
    assert_eq!(
        super::MutationRoute::for_slug(&records, "does-not-exist"),
        super::MutationRoute::Plain,
    );
}

/// `for_persona_d_tag` is the inbound routing key, and it is deliberately NOT
/// `for_slug`. A team-sourced projected persona derives its d-tag from
/// `source_team_persona_slug`, which differs from its UUID `slug`/`id`. The
/// inbound apply/tombstone arms match a local record by `persona_d_tag`, so the
/// preflight MUST route on the same derivation — a slug-only check would read
/// the very same inbound event as `Plain` and let it overwrite or delete the
/// projection. This pins that divergence: the projection is found by its d-tag
/// but NOT by its slug, and an unknown d-tag is a fresh insert (`Plain`).
#[test]
fn mutation_route_for_persona_d_tag_catches_projected_team_slug_that_for_slug_misses() {
    let mut record = projected_record("uuid-team-persona", 5);
    // Team-sourced: the d-tag derives from the pack slug, not the UUID id/slug.
    record.source_team_persona_slug = Some("codereviewer".to_string());
    let records = vec![record];

    // The inbound key (d-tag) finds the projection.
    assert_eq!(
        super::MutationRoute::for_persona_d_tag(&records, "codereviewer"),
        super::MutationRoute::LibraryProjected,
    );
    // The slug key does NOT — this is exactly why inbound must not use for_slug.
    assert_eq!(
        super::MutationRoute::for_slug(&records, "codereviewer"),
        super::MutationRoute::Plain,
    );
    // A d-tag matching no record is a fresh insert: Plain.
    assert_eq!(
        super::MutationRoute::for_persona_d_tag(&records, "unknown"),
        super::MutationRoute::Plain,
    );
}

/// §2.8 relation-resolver read side: `for_linked_definition` classifies an
/// instance's linkage by resolving its `persona_id` against the raw keyless
/// definition store. A `persona_id` pointing at a projected definition is
/// `LibraryProjected`; one pointing at a plain definition, at no definition (a
/// stale link), or `None` (a definition-less instance) is `Plain`. This is the
/// canonical join the inbound kind:30177 rule consults — the same `persona_id →
/// slug` derivation every library mechanism uses, resolved one way.
#[test]
fn for_linked_definition_classifies_projected_plain_absent_and_none() {
    let definitions = vec![
        projected_record("shared", 3),
        custom_persona("custom:plain", "Plain").into_agent_record(),
    ];

    // Instance linked to a projected definition → LibraryProjected.
    assert_eq!(
        super::MutationRoute::for_linked_definition(&definitions, Some("shared")),
        super::MutationRoute::LibraryProjected,
    );
    // Instance linked to a plain definition → Plain.
    assert_eq!(
        super::MutationRoute::for_linked_definition(&definitions, Some("custom:plain")),
        super::MutationRoute::Plain,
    );
    // Instance whose persona_id resolves to no definition (a stale link) → Plain.
    assert_eq!(
        super::MutationRoute::for_linked_definition(&definitions, Some("does-not-exist")),
        super::MutationRoute::Plain,
    );
    // Definition-less instance (`persona_id == None`) → Plain.
    assert_eq!(
        super::MutationRoute::for_linked_definition(&definitions, None),
        super::MutationRoute::Plain,
    );
}

/// §2.8 resolver vs. `persona_d_tag`: the instance→definition join keys on the
/// definition's UUID `slug` (== the instance `persona_id`), NOT the team-
/// derived d-tag. A team-sourced projected definition whose `persona_id`/`slug`
/// is a UUID but whose d-tag is the pack slug must still resolve as projected
/// through `for_linked_definition` — the resolver and the inbound d-tag
/// preflight key on different fields for different callers, and this pins that
/// the linkage join uses the slug.
#[test]
fn for_linked_definition_keys_on_slug_not_persona_d_tag() {
    let mut definition = projected_record("uuid-shared", 5);
    definition.source_team_persona_slug = Some("codereviewer".to_string());
    let definitions = vec![definition];

    // The instance's `persona_id` is the definition's UUID slug — resolves.
    assert_eq!(
        super::MutationRoute::for_linked_definition(&definitions, Some("uuid-shared")),
        super::MutationRoute::LibraryProjected,
    );
    // The team d-tag is NOT the slug — it does not resolve the linkage join.
    assert_eq!(
        super::MutationRoute::for_linked_definition(&definitions, Some("codereviewer")),
        super::MutationRoute::Plain,
    );
}

/// The fingerprint narrowing (F3): a plain persona save may freely change a
/// projected record's SCOPE-LOCAL fields (`is_active`, `env_vars`) — these are
/// not library-authoritative, so the shared fingerprint is unchanged and the
/// edit rides through the seam with the projection metadata intact. Only the
/// shared-definition slots are gated (see the reject test above). Without the
/// narrowing, a whole-record compare would wrongly block a legitimate local
/// activation toggle or env edit on a shared agent.
#[test]
fn merge_preserving_save_allows_scope_local_edit_on_projected_record() {
    let existing = vec![projected_record("shared", 3)];
    let mut view = existing[0].to_definition_view().expect("view");
    view.is_active = !view.is_active;
    view.env_vars =
        std::collections::BTreeMap::from([("API_KEY".to_string(), "value".to_string())]);

    let saved = merge_preserving_definitions(existing, &[view])
        .expect("a scope-local edit on a projection must pass the seam");
    assert_projection_survived(&saved, "scope_local_edit");

    let shared = saved
        .iter()
        .find(|record| record.slug.as_deref() == Some("shared"))
        .expect("projected record survives the scope-local edit");
    assert!(!shared.is_active, "activation toggle must be applied");
    assert_eq!(
        shared.env_vars.get("API_KEY").map(String::as_str),
        Some("value"),
        "env edit must be applied",
    );
}

/// The same guarantee through the on-disk `_at` seam — proving the storage
/// layer and the built-in-merge write-back preserve the metadata too — plus the
/// §2.7 read-side exposure (P4-C1): `load_persona_views_at` surfaces the
/// metadata for a projected record and reports none for a plain one.
#[test]
fn save_personas_at_preserves_library_metadata_on_disk() {
    let dir = tempfile::tempdir().expect("temp dir");
    let definitions_dir = dir.path();

    save_agent_definitions_at(
        definitions_dir,
        &[
            projected_record("shared", 3),
            custom_persona("custom:plain", "Plain").into_agent_record(),
        ],
    )
    .expect("seed raw store");

    // A full writer cycle: load the views (metadata stripped), edit the
    // unrelated persona, save back through the merge-preserving seam. The load
    // also merges built-ins and writes them back — another writer exercised.
    let mut views = load_personas_at(definitions_dir).expect("load personas");
    for view in &mut views {
        if view.id == "custom:plain" {
            view.display_name = "Renamed".to_string();
        }
    }
    save_personas_at(definitions_dir, &views).expect("save personas");

    let raw = load_agent_definitions_at(definitions_dir).expect("reload raw store");
    let projected = raw
        .iter()
        .find(|record| record.slug.as_deref() == Some("shared"))
        .expect("projected record survives on disk");
    assert_eq!(projected.library_ref.as_deref(), Some("lib-shared"));
    assert_eq!(projected.library_applied_revision, Some(3));

    let persona_views = load_persona_views_at(definitions_dir).expect("load persona views");
    let shared_view = persona_views
        .iter()
        .find(|view| view.definition.id == "shared")
        .expect("shared view present");
    assert_eq!(shared_view.library_ref.as_deref(), Some("lib-shared"));
    assert_eq!(shared_view.library_applied_revision, Some(3));

    let plain_view = persona_views
        .iter()
        .find(|view| view.definition.id == "custom:plain")
        .expect("plain view present");
    assert!(
        plain_view.library_ref.is_none() && plain_view.library_applied_revision.is_none(),
        "a plain persona must surface no library metadata",
    );
}

/// F3 per-command-path preflight (§2.7): every command boundary reads its route
/// from the RAW keyless store it loads, never from an in-memory vector or an
/// inbound/import view (`library_ref` is not view-carried). `delete_persona` and
/// both snapshot/team imports load `load_agent_definitions[_at]` then route on
/// `for_slug`; both inbound arms load the same store then route on
/// `for_persona_d_tag`. The command bodies take a concrete `AppHandle<Wry>`, so
/// they cannot be driven by the `MockRuntime` harness — this binds the decision
/// each one consults to the real serializer path instead: a projected keyless
/// definition must survive the `pubkey.is_empty()` definition filter AND the
/// JSON round-trip and STILL classify `LibraryProjected`, or a preflight would
/// wave a projected target onto the destructive plain path. A team-sourced
/// projection exercises the inbound d-tag key (derived from
/// `source_team_persona_slug`, not the UUID slug) end-to-end through disk. The
/// refusal-precedes-all-effects ordering is by construction at each call site
/// (verified by source inspection) — the preflight is the first statement under
/// the store lock, before any runtime stop, cascade delete, retention write, or
/// keyed-record save.
#[test]
fn command_preflight_routes_projected_record_off_the_reloaded_raw_store() {
    let dir = tempfile::tempdir().expect("temp dir");
    let definitions_dir = dir.path();

    let mut projected = projected_record("uuid-shared", 3);
    // Team-sourced: the inbound arms match by `persona_d_tag`, which derives
    // from the pack slug, not the UUID id/slug.
    projected.source_team_persona_slug = Some("codereviewer".to_string());
    save_agent_definitions_at(
        definitions_dir,
        &[
            projected,
            custom_persona("custom:plain", "Plain").into_agent_record(),
        ],
    )
    .expect("seed raw store");

    // The exact load every preflight runs (`load_agent_definitions[_at]` share
    // one body: read the store, retain keyless definitions). The projected
    // keyless definition must survive it, or the route would silently be Plain.
    let raw = load_agent_definitions_at(definitions_dir).expect("reload raw store");
    assert!(
        raw.iter().any(|r| r.slug.as_deref() == Some("uuid-shared")),
        "projected keyless definition must survive the definition filter",
    );

    // delete + snapshot/team import key: route on the UUID slug.
    assert_eq!(
        super::MutationRoute::for_slug(&raw, "uuid-shared"),
        super::MutationRoute::LibraryProjected,
        "delete/import preflight must refuse a projected slug",
    );
    assert_eq!(
        super::MutationRoute::for_slug(&raw, "custom:plain"),
        super::MutationRoute::Plain,
        "a plain persona is not a projection",
    );

    // inbound upsert/tombstone key: route on the team-derived d-tag. A slug-only
    // check would MISS it (the d-tag is not the UUID slug) — which is exactly
    // why the arms must use for_persona_d_tag.
    let projected_view = raw
        .iter()
        .find(|r| r.slug.as_deref() == Some("uuid-shared"))
        .and_then(|r| r.to_definition_view())
        .expect("projected view");
    let d_tag = crate::managed_agents::persona_events::persona_d_tag(&projected_view);
    assert_eq!(d_tag, "codereviewer");
    assert_eq!(
        super::MutationRoute::for_persona_d_tag(&raw, &d_tag),
        super::MutationRoute::LibraryProjected,
        "inbound preflight must refuse a projected target by its d-tag",
    );
    assert_eq!(
        super::MutationRoute::for_slug(&raw, &d_tag),
        super::MutationRoute::Plain,
        "the d-tag is not the slug — a slug-only inbound check would miss it",
    );
}
