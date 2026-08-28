use nostr::{EventBuilder, EventId, Kind};

use super::{check_content, tag};

/// Kind 30620 — replaceable workflow definition.
///
/// The `d` tag carries the workflow id; `h` tag carries the channel id; the
/// content is the YAML definition. Same (pubkey, d) replaces the prior version.
pub fn build_workflow_definition(
    workflow_id: &str,
    channel_id: &str,
    yaml_definition: &str,
    expected_revision: Option<&str>,
) -> Result<EventBuilder, String> {
    check_content(yaml_definition)?;
    let mut tags = vec![tag(vec!["d", workflow_id])?, tag(vec!["h", channel_id])?];
    if let Some(revision) = expected_revision {
        EventId::from_hex(revision).map_err(|_| "invalid workflow revision".to_string())?;
        tags.push(tag(vec!["expected-revision", revision])?);
    }
    Ok(EventBuilder::new(Kind::Custom(30620), yaml_definition.to_string()).tags(tags))
}

/// Kind 5 — NIP-09 deletion targeting a kind:30620 workflow definition.
pub fn build_workflow_delete(
    workflow_id: &str,
    owner_pubkey_hex: &str,
) -> Result<EventBuilder, String> {
    let coord = format!("30620:{owner_pubkey_hex}:{workflow_id}");
    let tags = vec![tag(vec!["a", &coord])?];
    Ok(EventBuilder::new(Kind::Custom(5), "").tags(tags))
}

/// Kind 46020 — trigger a workflow run by id, bound to one exact definition revision.
pub fn build_workflow_trigger(
    workflow_id: &str,
    definition_event_id: &str,
) -> Result<EventBuilder, String> {
    let workflow_id =
        uuid::Uuid::parse_str(workflow_id).map_err(|_| "invalid workflow id".to_string())?;
    buzz_sdk_pkg::build_workflow_trigger(workflow_id, definition_event_id)
        .map_err(|error| error.to_string())
}

/// Kind 46030 — grant an approval token (with optional note).
pub fn build_approval_grant(token: &str, note: Option<&str>) -> Result<EventBuilder, String> {
    let tags = vec![tag(vec!["t", token])?];
    Ok(EventBuilder::new(Kind::Custom(46030), note.unwrap_or("")).tags(tags))
}

/// Kind 46031 — deny an approval token (with optional note).
pub fn build_approval_deny(token: &str, note: Option<&str>) -> Result<EventBuilder, String> {
    let tags = vec![tag(vec!["t", token])?];
    Ok(EventBuilder::new(Kind::Custom(46031), note.unwrap_or("")).tags(tags))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_trigger_invocations_in_one_second_are_distinct() {
        let keys = nostr::Keys::generate();
        let workflow_id = uuid::Uuid::new_v4().to_string();
        let revision = "ab".repeat(32);
        let build = || {
            build_workflow_trigger(&workflow_id, &revision)
                .unwrap()
                .custom_created_at(nostr::Timestamp::from(1_700_000_000))
                .sign_with_keys(&keys)
                .unwrap()
        };
        assert_ne!(build().id, build().id);
        assert!(build_workflow_trigger(&workflow_id, "not-an-event-id").is_err());
    }
}
