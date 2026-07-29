use super::*;

#[test]
fn relink_persona_repairs_identity_without_reminting() {
    let persona = sample_persona();
    let mut record = persona.clone().into_agent_record();
    record.pubkey = "existing-agent-pubkey".to_string();
    record.slug = None;
    record.persona_id = Some("deleted-persona".to_string());
    record.persona_source_version = None;

    relink_persona(&mut record, &persona);

    assert_eq!(record.pubkey, "existing-agent-pubkey");
    assert_eq!(record.persona_id.as_deref(), Some("test-persona"));
    assert_eq!(
        record.system_prompt.as_deref(),
        Some("You are a test assistant.")
    );
    assert!(record.persona_source_version.is_some());
}

#[test]
fn detach_persona_materializes_runnable_standalone_snapshot() {
    let mut persona = sample_persona();
    persona
        .env_vars
        .insert("INHERITED".to_string(), "materialized".to_string());
    let mut record = persona.clone().into_agent_record();
    record.pubkey = "existing-agent-pubkey".to_string();
    record.slug = None;
    record.persona_id = Some(persona.id.clone());
    record.env_vars.clear();
    record
        .env_vars
        .insert("KEY".to_string(), "instance-override".to_string());
    record
        .env_vars
        .insert("INSTANCE_ONLY".to_string(), "yes".to_string());

    detach_persona(&mut record, &persona);

    assert_eq!(record.pubkey, "existing-agent-pubkey");
    assert_eq!(record.persona_id, None);
    assert_eq!(record.persona_source_version, None);
    assert_eq!(record.runtime.as_deref(), Some("goose"));
    assert_eq!(record.model.as_deref(), Some("claude-opus-4"));
    assert_eq!(record.provider.as_deref(), Some("anthropic"));
    assert_eq!(
        record.env_vars.get("KEY").map(String::as_str),
        Some("instance-override"),
        "instance override must win over inherited definition env"
    );
    assert_eq!(
        record.env_vars.get("INSTANCE_ONLY").map(String::as_str),
        Some("yes")
    );
    assert_eq!(
        record.env_vars.get("INHERITED").map(String::as_str),
        Some("materialized"),
        "inherited env must survive after the definition is removed"
    );
}
