use super::{create_time_agent_command_override, persona_with_runtime};

#[test]
fn preserves_deliberate_override_of_installed_runtime() {
    let personas = vec![persona_with_runtime("p1", Some("claude"))];
    assert_eq!(
        create_time_agent_command_override(Some("p1"), &personas, Some("codex-acp"), true),
        Some("codex-acp".to_string())
    );
}

#[test]
fn pins_visible_fallback_for_runtime_less_persona() {
    let personas = vec![persona_with_runtime("p1", None)];
    assert_eq!(
        create_time_agent_command_override(Some("p1"), &personas, Some("goose"), true),
        Some("goose".to_string())
    );
}
