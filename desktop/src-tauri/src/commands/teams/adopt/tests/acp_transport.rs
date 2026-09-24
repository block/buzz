use super::*;

#[test]
fn signed_team_transport_survives_adoption_and_next_spawn_snapshot() {
    let mut definition = persona("m1", "Do the work.");
    definition.acp_command = Some("buzz-janet-acp".to_string());
    let (event, source) = published(&team_fixture(vec!["m1".to_string()]), &[definition], true);
    let content = verified_head_content(&event, &source, &event.id.to_hex()).unwrap();
    let (personas, _) = plan_add(&[], &[], &source, &content, NOW)
        .unwrap()
        .stores
        .unwrap();
    assert_eq!(personas[0].acp_command.as_deref(), Some("buzz-janet-acp"));
    let mut instance = personas[0].clone().into_agent_record();
    instance.acp_command = "buzz-acp".to_string();
    crate::managed_agents::persona_events::apply_persona_snapshot(&mut instance, &personas[0]);
    assert_eq!(instance.acp_command, "buzz-janet-acp");
}
