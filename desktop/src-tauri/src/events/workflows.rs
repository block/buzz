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

/// Kind 46020 — trigger a workflow run by id.
pub fn build_workflow_trigger(workflow_id: &str) -> Result<EventBuilder, String> {
    let tags = vec![tag(vec!["d", workflow_id])?];
    Ok(EventBuilder::new(Kind::Custom(46020), "").tags(tags))
}

/// Kind 46030 — grant an approval token (with optional note).
pub fn build_approval_grant(token: &str, note: Option<&str>) -> Result<EventBuilder, String> {
    build_approval_action(46030, token, note)
}

/// Kind 46031 — deny an approval token (with optional note).
pub fn build_approval_deny(token: &str, note: Option<&str>) -> Result<EventBuilder, String> {
    build_approval_action(46031, token, note)
}

fn build_approval_action(
    kind: u16,
    approval_ref: &str,
    note: Option<&str>,
) -> Result<EventBuilder, String> {
    // The relay resolves approval actions from an `e` (or legacy `d`) tag.
    // Approval references are SHA-256 hashes, so validating them as event IDs
    // both enforces the 32-byte wire shape and gives us a standard Nostr tag.
    EventId::from_hex(approval_ref).map_err(|_| "invalid approval reference".to_string())?;
    let tags = vec![tag(vec!["e", approval_ref])?];
    Ok(EventBuilder::new(Kind::Custom(kind), note.unwrap_or("")).tags(tags))
}

#[cfg(test)]
mod tests {
    use nostr::Keys;

    use super::*;

    const APPROVAL_REF: &str = "abababababababababababababababababababababababababababababababab";

    #[test]
    fn approval_actions_use_the_reference_tag_consumed_by_the_relay() {
        for builder in [
            build_approval_grant(APPROVAL_REF, Some("approved")).expect("grant builder"),
            build_approval_deny(APPROVAL_REF, Some("denied")).expect("deny builder"),
        ] {
            let event = builder
                .sign_with_keys(&Keys::generate())
                .expect("sign approval action");
            let reference = event.tags.iter().find_map(|tag| {
                (tag.kind().to_string() == "e")
                    .then(|| tag.content().map(str::to_string))
                    .flatten()
            });

            assert_eq!(reference.as_deref(), Some(APPROVAL_REF));
            assert!(!event.tags.iter().any(|tag| tag.kind().to_string() == "t"));
        }
    }

    #[test]
    fn approval_actions_reject_malformed_references() {
        assert!(build_approval_grant("not-a-hash", None).is_err());
        assert!(build_approval_deny(&"ab".repeat(31), None).is_err());
    }
}
