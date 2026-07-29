use nostr::Tag;

pub(super) fn push_agent_response_tag(
    tags: &mut Vec<Tag>,
    policy: Option<&str>,
) -> Result<(), String> {
    let Some(policy) = policy else {
        return Ok(());
    };
    if !matches!(policy, "mentions" | "all") {
        return Err("agent_response must be \"mentions\" or \"all\"".into());
    }
    tags.push(Tag::parse(vec!["agent_response", policy]).map_err(|e| format!("invalid tag: {e}"))?);
    Ok(())
}
