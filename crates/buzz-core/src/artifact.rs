//! NIP-AR envelope validation; client-defined payloads and annotations are opaque.
use nostr::Event;
use uuid::Uuid;
/// Maximum artifact tags, including the envelope.
pub const MAX_TAGS: usize = 256;
/// Maximum UTF-8 bytes in a tag name.
pub const MAX_TAG_NAME_BYTES: usize = 128;
/// Maximum UTF-8 bytes in each tag value.
pub const MAX_TAG_VALUE_BYTES: usize = 4096;
/// Maximum UTF-8 bytes across all tags.
pub const MAX_TAG_BYTES: usize = 65536;
/// Maximum predicates per artifact query.
pub const MAX_PREDICATES: usize = 32;
/// Maximum total requested predicate values.
pub const MAX_QUERY_VALUES: usize = 256;
/// Maximum artifact page size.
pub const MAX_PAGE_SIZE: usize = 1000;
/// Lifecycle operation named by a revision's `op` tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactOp {
    /// First revision of a new identity.
    Create,
    /// Content change within the same home.
    Update,
    /// Content snapshot published into a new home.
    Move,
    /// Soft delete; only `Restore` may follow.
    Delete,
    /// Complete snapshot that revives a deleted artifact.
    Restore,
}
impl ArtifactOp {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "create" => Self::Create,
            "update" => Self::Update,
            "move" => Self::Move,
            "delete" => Self::Delete,
            "restore" => Self::Restore,
            _ => return None,
        })
    }
}
/// The authoritative envelope, independent of any client's content schema.
#[derive(Debug)]
pub struct ArtifactEnvelope {
    /// Community-local stable identity.
    pub id: Uuid,
    /// Home channel.
    pub home: Uuid,
    /// Immutable type name.
    pub artifact_type: String,
    /// Lifecycle operation.
    pub op: ArtifactOp,
    /// Expected previous revision, absent on create.
    pub prev: Option<Vec<u8>>,
    /// Optional conversation anchor.
    pub root: Option<Vec<u8>>,
}
/// Parse a canonical, non-nil UUID.
pub fn canonical_uuid(value: &str) -> Result<Uuid, &'static str> {
    let id = Uuid::parse_str(value).map_err(|_| "invalid UUID")?;
    if id.is_nil() || id.to_string() != value {
        return Err("UUID must be canonical lowercase and non-nil");
    }
    Ok(id)
}
/// Parse a canonical Nostr event identifier.
pub fn event_id(value: &str) -> Result<Vec<u8>, &'static str> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("event ID must be 64 lowercase hex characters");
    }
    hex::decode(value).map_err(|_| "invalid event ID")
}
/// Validate the complete envelope, leaving auth-tag verification to the relay.
pub fn validate(event: &Event) -> Result<ArtifactEnvelope, &'static str> {
    if event.kind.as_u16() != 45010 {
        return Err("not an artifact revision");
    }
    if event.tags.len() > MAX_TAGS {
        return Err("too many tags");
    }
    let names = ["ar", "d", "h", "type", "title", "op", "root", "prev"];
    let mut fields = std::collections::HashMap::new();
    let mut bytes = 0;
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        let Some(name) = parts.first() else {
            return Err("empty tag");
        };
        if name.len() > MAX_TAG_NAME_BYTES {
            return Err("tag name too long");
        }
        for value in parts {
            bytes += value.len();
            if value.len() > MAX_TAG_VALUE_BYTES {
                return Err("tag value too long");
            }
        }
        if names.contains(&name.as_str())
            && (parts.len() != 2 || fields.insert(name.as_str(), parts[1].as_str()).is_some())
        {
            return Err("envelope tags must occur once with exactly two elements");
        }
    }
    if bytes > MAX_TAG_BYTES {
        return Err("total tag bytes exceeded");
    }
    let field = |name| {
        fields
            .get(name)
            .copied()
            .ok_or("missing required envelope tag")
    };
    if field("ar")? != "1" {
        return Err("unsupported artifact envelope version");
    }
    let id = canonical_uuid(field("d")?)?;
    let home = canonical_uuid(field("h")?)?;
    let artifact_type = field("type")?;
    if artifact_type.len() > 128
        || !artifact_type.contains('.')
        || !artifact_type.split('.').all(|c| {
            !c.is_empty()
                && c.as_bytes()[0].is_ascii_lowercase()
                && c.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        })
    {
        return Err("invalid namespaced artifact type");
    }
    let op = ArtifactOp::parse(field("op")?).ok_or("invalid artifact operation")?;
    let prev = fields.get("prev").map(|s| event_id(s)).transpose()?;
    if (op == ArtifactOp::Create) != prev.is_none() {
        return Err("prev required exactly on non-create revisions");
    }
    let root = fields.get("root").map(|s| event_id(s)).transpose()?;
    if op == ArtifactOp::Delete {
        if fields.contains_key("title") || !event.content.is_empty() {
            return Err("delete must omit title and have empty content");
        }
        if event
            .tags
            .iter()
            .any(|t| !names.contains(&t.as_slice()[0].as_str()) && t.as_slice()[0] != "auth")
        {
            return Err("delete allows only envelope and verified auth tags");
        }
    } else {
        let title = field("title")?;
        if title.trim().is_empty() || title.len() > 512 {
            return Err("title must be nonblank and at most 512 UTF-8 bytes");
        }
    }
    Ok(ArtifactEnvelope {
        id,
        home,
        artifact_type: artifact_type.into(),
        op,
        prev,
        root,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    fn event(extra: Vec<Vec<String>>, op: &str, content: &str) -> Event {
        let mut tags = vec![
            vec!["ar".into(), "1".into()],
            vec!["d".into(), Uuid::new_v4().to_string()],
            vec!["h".into(), Uuid::new_v4().to_string()],
            vec!["type".into(), "buzz.task".into()],
            vec!["op".into(), op.into()],
        ];
        if op != "delete" {
            tags.push(vec!["title".into(), "Title".into()]);
        }
        if op != "create" {
            tags.push(vec!["prev".into(), "a".repeat(64)]);
        }
        tags.extend(extra);
        EventBuilder::new(Kind::Custom(45010), content)
            .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }
    #[test]
    fn lifecycle_envelopes() {
        for op in ["create", "update", "move", "delete", "restore"] {
            assert!(validate(&event(vec![], op, "")).is_ok(), "{op}");
        }
        assert!(validate(&event(
            vec![vec!["project".into(), "opaque".into(), "anything".into()]],
            "create",
            "not json"
        ))
        .is_ok());
    }
    #[test]
    fn malformed_and_delete_payloads() {
        for (tags, op, content) in [
            (vec![vec!["title".into(), "duplicate".into()]], "create", ""),
            (vec![vec!["ar".into(), "2".into()]], "create", ""),
            (vec![vec!["prev".into(), "a".repeat(64)]], "create", ""),
            (vec![vec!["root".into(), "A".repeat(64)]], "create", ""),
            (vec![vec!["project".into(), "hidden".into()]], "delete", ""),
            (vec![vec!["title".into(), "hidden".into()]], "delete", ""),
            (vec![], "delete", "hidden"),
            (vec![vec!["x".into(), "x".repeat(4097)]], "create", ""),
            (vec![vec!["x".repeat(129), "x".into()]], "create", ""),
        ] {
            assert!(validate(&event(tags, op, content)).is_err());
        }
    }
    #[test]
    fn filter_routes_and_views() {
        use serde_json::json;
        assert_eq!(
            route_filter(&json!({"kinds":[30621],"#buzz-channel":["c"]})),
            FilterRoute::Generic
        );
        assert_eq!(
            route_filter(&json!({"kinds":[45010,45011],"#h":["c"]})),
            FilterRoute::Generic
        );
        for rejected in [
            json!({"kinds":[45010],"#project":["p"]}),
            json!({"ids":["a"],"#project":["p"]}),
            json!({"kinds":"x","#project":["p"]}),
        ] {
            assert!(
                matches!(route_filter(&rejected), FilterRoute::Rejected(_)),
                "{rejected}"
            );
        }
        assert_eq!(
            route_filter(&json!({"ids":["a"],"#h":["c"]})),
            FilterRoute::Generic
        );
        assert_eq!(
            route_filter(&json!({"artifact":"current"})),
            FilterRoute::Artifact
        );
        let query = parse_query(&json!({"artifact":"current","#project":["p"]})).unwrap();
        assert_eq!(query.view, ArtifactView::Current);
        for invalid in [
            json!({"artifact":"lookup","#d":["x"]}),
            json!({"artifact":"history"}),
            json!({"artifact":"current","kinds":[45011]}),
            json!({"artifact":"current","search":"x"}),
            json!({"artifact":"current","limit":1001}),
            json!({"artifact":"current","#assignee":[]}),
            json!({"artifact":"history","#d":[]}),
        ] {
            assert!(parse_query(&invalid).is_err(), "{invalid}");
        }
    }
    #[test]
    fn canonical_identifiers() {
        assert!(canonical_uuid("00000000-0000-0000-0000-000000000000").is_err());
        assert!(canonical_uuid(&Uuid::new_v4().simple().to_string()).is_err());
        assert!(event_id(&"f".repeat(64)).is_ok());
        assert!(event_id(&"F".repeat(64)).is_err());
    }
}

/// Which artifact revisions an explicit query reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactView {
    /// Each live artifact's current revision; deleted artifacts are omitted.
    Current,
    /// Every stored, unredacted revision of the artifacts named by `#d`.
    History,
}
/// Explicit HTTP artifact query. All `#name` predicates compare the first
/// value of the same tag.
#[derive(Debug)]
pub struct ArtifactQuery {
    /// Query view.
    pub view: ArtifactView,
    /// Exact tag-name/value predicates.
    pub tags: Vec<(String, Vec<String>)>,
    /// Maximum result count.
    pub limit: i64,
    /// Bounded page offset.
    pub offset: i64,
}
/// Maximum artifact page offset.
pub const MAX_OFFSET: u64 = 10000;
/// How a raw REQ/COUNT filter must be served.
#[derive(Debug, PartialEq, Eq)]
pub enum FilterRoute {
    /// Standard Nostr filter handling.
    Generic,
    /// Explicit artifact query (`artifact` key present).
    Artifact,
    /// Would silently drop predicates on the generic path.
    Rejected(&'static str),
}
/// Route a raw filter. Generic filters drop multi-character tag predicates,
/// so filters that may match artifacts (including those without `kinds`)
/// carrying them must use an explicit artifact query.
pub fn route_filter(value: &serde_json::Value) -> FilterRoute {
    if value.get("artifact").is_some() {
        return FilterRoute::Artifact;
    }
    let artifact_kind = value.get("kinds").is_none_or(|k| {
        k.as_array()
            .is_none_or(|ks| ks.iter().any(|k| matches!(k.as_u64(), Some(45010 | 45011))))
    });
    let multi_character = value
        .as_object()
        .is_some_and(|o| o.keys().any(|k| k.starts_with('#') && k.len() > 2));
    if artifact_kind && multi_character {
        return FilterRoute::Rejected(
            "multi-character tag predicates on artifacts require an artifact query",
        );
    }
    FilterRoute::Generic
}
/// Parse an artifact filter, explicitly rejecting unsupported predicates.
pub fn parse_query(value: &serde_json::Value) -> Result<ArtifactQuery, &'static str> {
    let object = value
        .as_object()
        .ok_or("artifact filter must be an object")?;
    let view = match object.get("artifact").and_then(|v| v.as_str()) {
        Some("current") => ArtifactView::Current,
        Some("history") => ArtifactView::History,
        _ => return Err("artifact must be \"current\" or \"history\""),
    };
    if let Some(kinds) = object.get("kinds") {
        if kinds.as_array().map(Vec::as_slice) != Some(&[serde_json::json!(45010)]) {
            return Err("artifact queries accept only kinds [45010]");
        }
    }
    let mut tags = Vec::new();
    let mut total = 0;
    let mut bytes = 0;
    for (name, value) in object {
        if let Some(name) = name.strip_prefix('#') {
            if name.is_empty() || name.len() > MAX_TAG_NAME_BYTES {
                return Err("invalid predicate name size");
            }
            let values = value
                .as_array()
                .filter(|v| !v.is_empty())
                .ok_or("predicate values must be non-empty arrays")?;
            let mut parsed = Vec::new();
            for value in values {
                let value = value.as_str().ok_or("predicate values must be strings")?;
                if value.len() > MAX_TAG_VALUE_BYTES {
                    return Err("predicate value too large");
                }
                bytes += name.len() + value.len();
                total += 1;
                parsed.push(value.into());
            }
            tags.push((name.into(), parsed));
        } else if !["artifact", "kinds", "limit", "offset"].contains(&name.as_str()) {
            return Err("unsupported artifact predicate");
        }
    }
    if tags.len() > MAX_PREDICATES || total > MAX_QUERY_VALUES || bytes > MAX_TAG_BYTES {
        return Err("artifact predicate limits exceeded");
    }
    let number = |name: &str, default: u64| {
        object
            .get(name)
            .map(|v| v.as_u64().ok_or("invalid limit or offset"))
            .transpose()
            .map(|v| v.unwrap_or(default))
    };
    let limit = number("limit", 100)?;
    let offset = number("offset", 0)?;
    if limit == 0 || limit > MAX_PAGE_SIZE as u64 || offset > MAX_OFFSET {
        return Err("artifact page limit exceeded");
    }
    if view == ArtifactView::History && !tags.iter().any(|(n, _)| n == "d") {
        return Err("history requires #d");
    }
    Ok(ArtifactQuery {
        view,
        tags,
        limit: limit as i64,
        offset: offset as i64,
    })
}
