//! NIP-AR sync filter: `{"artifact":"current", "#h":[...], ...}` on a `REQ`.
//! The same current-state filter as HTTP `/query`; the `REQ` transport makes
//! it a sync (complete set, `EOSE`, then live heads).
use crate::artifact::{canonical_uuid, parse_predicates};
use uuid::Uuid;

/// Largest set one sync serves: it bounds each initial read and the held
/// list of changed artifacts. Advertised in NIP-11 as `sync.max_events`.
pub const MAX_SYNC_EVENTS: usize = 4096;

/// A parsed sync filter. `channels` are the `#h` values; `tags` holds every
/// `#name` predicate, `#h` included, with NIP-AR match semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactSyncFilter {
    /// Named channels (`#h`), deduplicated in request order. Never empty.
    pub channels: Vec<Uuid>,
    /// Exact tag-name/value predicates (AND across names, OR within one).
    pub tags: Vec<(String, Vec<String>)>,
}

/// Tags NIP-AR fixes at creation. A head that fails a predicate on one of
/// them was never in scope and never will be.
const IMMUTABLE_TAGS: [&str; 2] = ["d", "type"];

impl ArtifactSyncFilter {
    /// Whether `head` fails a predicate on an immutable tag, so no change to
    /// it can be in scope (NIP-AR sync rule 3). Mutable tags are left to the
    /// client's match rule: a head that stopped matching must still be sent.
    pub fn never_in_scope(&self, head: &nostr::Event) -> bool {
        self.tags
            .iter()
            .filter(|(name, _)| IMMUTABLE_TAGS.contains(&name.as_str()))
            .any(|(name, values)| {
                let first = head.tags.iter().find_map(|tag| match tag.as_slice() {
                    [tag_name, value, ..] if tag_name == name => Some(value),
                    _ => None,
                });
                !first.is_some_and(|value| values.contains(value))
            })
    }
}

/// Whether a raw `REQ` filter is a sync: the current-state view. Other
/// artifact views have no live form and stay HTTP-only.
pub fn is_sync_filter(value: &serde_json::Value) -> bool {
    value.get("artifact").and_then(|v| v.as_str()) == Some("current")
}

/// Parse a sync filter. It requires `#h` with canonical channel UUIDs and
/// accepts the current-state predicates and `kinds: [45010]`, but no `limit`
/// or `offset`: a sync always serves the whole set.
pub fn parse_sync(value: &serde_json::Value) -> Result<ArtifactSyncFilter, &'static str> {
    let object = value
        .as_object()
        .ok_or("artifact filter must be an object")?;
    if !is_sync_filter(value) {
        return Err("artifact must be \"current\"");
    }
    let tags = parse_predicates(object, &["artifact", "kinds"])?;
    let mut channels = Vec::new();
    for (_, values) in tags.iter().filter(|(name, _)| name == "h") {
        for value in values {
            let id = canonical_uuid(value).map_err(|_| "#h values must be channel UUIDs")?;
            if !channels.contains(&id) {
                channels.push(id);
            }
        }
    }
    if channels.is_empty() {
        return Err("sync requires #h");
    }
    Ok(ArtifactSyncFilter { channels, tags })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sync_filter_requires_channels_and_rejects_paging() {
        let a = Uuid::new_v4().to_string();
        let parsed = parse_sync(
            &json!({"artifact":"current","#h":[a, a],"#type":["buzz.task"],"kinds":[45010]}),
        )
        .unwrap();
        assert_eq!(parsed.channels.len(), 1);
        assert_eq!(parsed.tags.len(), 2);
        for invalid in [
            json!({"artifact":"current"}),
            json!({"artifact":"current","#type":["buzz.task"]}),
            json!({"artifact":"current","#h":[a],"limit":10}),
            json!({"artifact":"current","#h":[a],"offset":0}),
            json!({"artifact":"current","#h":[a],"kinds":[45011]}),
            json!({"artifact":"current","#h":["not-a-uuid"]}),
            json!({"artifact":"current","#h":[a],"since":1}),
            json!({"artifact":"history","#h":[a]}),
        ] {
            assert!(parse_sync(&invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn never_in_scope_checks_only_immutable_tags() {
        let h = Uuid::new_v4().to_string();
        let filter = parse_sync(
            &json!({"artifact":"current","#h":[h],"#type":["work.set"],"#status":["open"]}),
        )
        .unwrap();
        let keys = nostr::Keys::generate();
        let head = |tags: &[[&str; 2]]| {
            nostr::EventBuilder::new(nostr::Kind::Custom(45010), "")
                .tags(tags.iter().map(|t| nostr::Tag::parse(t.to_vec()).unwrap()))
                .sign_with_keys(&keys)
                .unwrap()
        };
        // Another type: never in scope, whatever its other tags.
        assert!(filter.never_in_scope(&head(&[
            ["h", &h],
            ["type", "work.goals"],
            ["status", "open"]
        ])));
        assert!(filter.never_in_scope(&head(&[["h", &h]])));
        // Same type but a mutable tag no longer matches: still sent, so the
        // client can drop it.
        assert!(!filter.never_in_scope(&head(&[
            ["h", &h],
            ["type", "work.set"],
            ["status", "done"]
        ])));
        assert!(!filter.never_in_scope(&head(&[["h", &h], ["type", "work.set"]])));
    }
}
