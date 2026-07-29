use nostr::Event;

pub(super) fn from_event(event: &Event) -> String {
    event
        .tags
        .iter()
        .find_map(|tag| match tag.as_slice() {
            [name, value, ..]
                if name == "agent_response" && matches!(value.as_str(), "mentions" | "all") =>
            {
                Some(value.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| "mentions".to_string())
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind, Tag};

    use super::from_event;

    fn event(policy: Option<&str>) -> nostr::Event {
        let mut tags = vec![Tag::parse(["d", "channel-id"]).unwrap()];
        if let Some(policy) = policy {
            tags.push(Tag::parse(["agent_response", policy]).unwrap());
        }
        EventBuilder::new(Kind::Custom(39000), "")
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }

    #[test]
    fn reads_valid_policy_and_defaults_invalid_or_missing_values() {
        assert_eq!(from_event(&event(Some("all"))), "all");
        assert_eq!(from_event(&event(Some("sometimes"))), "mentions");
        assert_eq!(from_event(&event(None)), "mentions");
    }
}
