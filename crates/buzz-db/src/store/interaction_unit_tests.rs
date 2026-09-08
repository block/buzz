use super::*;

#[test]
fn text_projection_preserves_the_asker_for_agent_authorization() {
    let asker = Keys::generate();
    let relay = Keys::generate();
    let agent = Keys::generate().public_key().to_hex();
    let channel = Uuid::new_v4().to_string();
    let prompt = EventBuilder::new(Kind::Custom(KIND_INTERACTION_PROMPT as u16), "Approve?")
        .tags([
            Tag::parse(["h", &channel]).unwrap(),
            Tag::parse(["p", &agent]).unwrap(),
            Tag::parse(["itype", "buttons"]).unwrap(),
            Tag::parse(["opt", "approve", "Approve"]).unwrap(),
            Tag::parse(["closes", "first"]).unwrap(),
            Tag::parse(["deadline", "2000000000"]).unwrap(),
        ])
        .sign_with_keys(&asker)
        .unwrap();
    let schema = Prompt::parse(&prompt).unwrap();
    let message = projection(&prompt, &schema, &relay).unwrap();
    message.verify().unwrap();
    assert_eq!(message.pubkey, relay.public_key());
    assert_eq!(
        interaction::single_tag(&message, "actor").unwrap(),
        Some(asker.public_key().to_hex().as_str())
    );
    assert_eq!(
        interaction::single_tag(&message, "interaction").unwrap(),
        Some(prompt.id.to_hex().as_str())
    );
    assert_eq!(
        interaction::single_tag(&message, "p").unwrap(),
        Some(agent.as_str())
    );
}
