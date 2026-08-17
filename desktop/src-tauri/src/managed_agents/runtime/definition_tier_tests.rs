//! Definition-tier fail-closed tests: verify that a linked instance's spawn
//! and readiness gate correctly refuse when the definition's secrets are
//! unavailable (env_vars_ref present but could not be hydrated from keyring).
use super::test_fixtures::fixture;
use crate::managed_agents::types::{ManagedAgentRecord, RespondTo};

fn pin_persona(record: &mut ManagedAgentRecord, persona: &crate::managed_agents::AgentDefinition) {
    record.persona_id = Some(persona.id.clone());
}

fn persona_v(
    id: &str,
    prompt: &str,
    env: &[(&str, &str)],
) -> crate::managed_agents::AgentDefinition {
    use std::collections::BTreeMap;
    crate::managed_agents::AgentDefinition {
        id: id.to_string(),
        display_name: id.to_string(),
        avatar_url: None,
        system_prompt: prompt.to_string(),
        runtime: Some("goose".to_string()),
        model: None,
        provider: None,
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        env_vars: env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<_, _>>(),
        respond_to: None,
        respond_to_allowlist: vec![],
        parallelism: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        secrets_unavailable: false,
    }
}

/// The pure definition-unavailable predicate that `spawn_agent_child`,
/// `runtime_status`, and `unkeyable_failed_status` all consult before reaching
/// any AppHandle-dependent path.
///
/// `spawn_agent_child` gates on:
///   if let Some(pid) = unavailable_definition_id(record, personas) { → Err }
///
/// This mirrors the test shape of `orphaned_linked_instance_returns_error` in
/// the same file: we test the predicate directly without a full AppHandle.
#[test]
fn definition_unavailable_spawn_predicate_fires() {
    // A definition whose env_vars_ref could not be hydrated carries
    // secrets_unavailable=true. Any linked instance must be refused at spawn.
    let mut def = persona_v("def-slug", "prompt", &[("ANTHROPIC_API_KEY", "secret")]);
    def.secrets_unavailable = true;

    let mut record = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    pin_persona(&mut record, &def);
    assert_eq!(record.persona_id.as_deref(), Some("def-slug"));

    let personas = std::slice::from_ref(&def);
    assert_eq!(
        crate::managed_agents::unavailable_definition_id(&record, personas),
        Some("def-slug"),
        "spawn must detect unavailable definition and name it via the shared predicate"
    );
}

#[test]
fn definition_available_spawn_predicate_does_not_fire() {
    // Same setup but the definition is available — the predicate must return None.
    let mut def = persona_v("def-slug", "prompt", &[("ANTHROPIC_API_KEY", "secret")]);
    def.secrets_unavailable = false;

    let mut record = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    pin_persona(&mut record, &def);

    let personas = std::slice::from_ref(&def);
    assert_eq!(
        crate::managed_agents::unavailable_definition_id(&record, personas),
        None,
        "spawn must not refuse a linked instance whose definition is available"
    );
}

#[test]
fn unlinked_record_never_matches_a_definition() {
    // A record with no persona_id is not linked to any definition; even an
    // unavailable definition in the slice must not make the predicate fire.
    let def = {
        let mut d = persona_v("def-slug", "prompt", &[("ANTHROPIC_API_KEY", "secret")]);
        d.secrets_unavailable = true;
        d
    };
    let mut record = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    record.persona_id = None;

    assert_eq!(
        crate::managed_agents::unavailable_definition_id(&record, std::slice::from_ref(&def)),
        None,
        "an unlinked record must never match a definition's unavailability"
    );
}

#[test]
fn linked_definition_absent_from_slice_yields_none() {
    // A record linked to a persona that is not present in the slice must yield
    // None rather than treating the missing definition as unavailable.
    let unavailable_other = persona_v(
        "some-other-slug",
        "prompt",
        &[("ANTHROPIC_API_KEY", "secret")],
    );
    let mut record = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    record.persona_id = Some("linked-but-missing".to_string());

    assert_eq!(
        crate::managed_agents::unavailable_definition_id(
            &record,
            std::slice::from_ref(&unavailable_other)
        ),
        None,
        "a definition absent from the slice must not fire the predicate"
    );
}

#[test]
fn definition_unavailable_propagates_via_to_definition_view() {
    // `to_definition_view` must carry the definition record's
    // secrets_unavailable flag into the AgentDefinition shape so that
    // spawn/readiness callers get accurate availability state.
    let mut def_record: ManagedAgentRecord = serde_json::from_str(
        r#"{
            "pubkey": "",
            "relay_url": "",
            "slug": "def-slug",
            "name": "Def",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "system_prompt": "p",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }"#,
    )
    .expect("definition fixture");
    def_record.secrets_unavailable = true;

    let view = def_record
        .to_definition_view()
        .expect("definition must produce a view");
    assert!(
        view.secrets_unavailable,
        "to_definition_view must propagate secrets_unavailable from the record"
    );

    // And the inverse: available record → available view.
    def_record.secrets_unavailable = false;
    let view2 = def_record.to_definition_view().expect("second view");
    assert!(
        !view2.secrets_unavailable,
        "to_definition_view must propagate false when the definition is available"
    );
}
