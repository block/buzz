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

/// NIP-AR `type` of a factory-generated, hash-bound HTML review revision.
pub const SYNAXIS_REVIEW_TYPE: &str = "synaxis.html-review";
/// NIP-AR `type` of human feedback bound to one exact review revision.
pub const SYNAXIS_FEEDBACK_TYPE: &str = "synaxis.artifact-feedback";
const SYNAXIS_FEEDBACK_SCHEMA: &str = "synaxis.artifact-feedback/v1";
const MAX_FEEDBACK_REQUEST_BYTES: usize = 8 * 1024;

/// The review revision a `synaxis.artifact-feedback/v1` create names.
#[derive(Debug, PartialEq, Eq)]
pub struct FeedbackTarget {
    /// Stable NIP-AR UUID of the reviewed `synaxis.html-review` artifact.
    pub review_artifact: Uuid,
    /// Event ID the feedback was written against.
    pub revision: Vec<u8>,
    /// Synaxis artifact ID the reviewed revision carries.
    pub synaxis_artifact_id: String,
    /// Payload digest the reviewed revision carries.
    pub payload_digest: String,
}

fn single_tag<'a>(event: &'a Event, name: &str) -> Result<&'a str, &'static str> {
    let mut matches = event.tags.iter().filter(|t| t.as_slice()[0] == name);
    match (matches.next(), matches.next()) {
        (Some(tag), None) if tag.as_slice().len() == 2 => Ok(tag.as_slice()[1].as_str()),
        _ => Err("feedback filter tags must occur once with two elements"),
    }
}

fn hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Validate a `synaxis.artifact-feedback/v1` revision. Feedback is a
/// create-only record: any other operation is refused, and the filterable
/// `target`, `target_revision`, `synaxis_artifact`, and `payload` tags must
/// restate the signed content exactly so a tag can never redirect feedback.
pub fn validate_feedback_create(
    event: &Event,
    env: &ArtifactEnvelope,
) -> Result<FeedbackTarget, &'static str> {
    if env.op != ArtifactOp::Create {
        return Err("synaxis feedback is create-only");
    }
    let content: serde_json::Value =
        serde_json::from_str(&event.content).map_err(|_| "feedback content is not JSON")?;
    if content.get("schema").and_then(|v| v.as_str()) != Some(SYNAXIS_FEEDBACK_SCHEMA) {
        return Err("unsupported feedback schema");
    }
    fn text(value: Option<&serde_json::Value>) -> Option<&str> {
        value.and_then(|v| v.as_str())
    }
    let reviewed = content.get("reviewed");
    let review_artifact = text(reviewed.and_then(|r| r.get("buzz_artifact_id")))
        .ok_or("feedback names no reviewed artifact")
        .and_then(canonical_uuid)?;
    let revision = text(reviewed.and_then(|r| r.get("buzz_revision_event_id")))
        .filter(|v| hex64(v))
        .ok_or("feedback names no reviewed revision")?;
    let synaxis_artifact_id = text(reviewed.and_then(|r| r.get("synaxis_artifact_id")))
        .filter(|v| !v.is_empty() && v.len() <= 128)
        .ok_or("feedback names no Synaxis artifact")?;
    let payload_digest = text(reviewed.and_then(|r| r.get("payload_digest")))
        .filter(|v| hex64(v))
        .ok_or("feedback names no payload digest")?;
    let block = text(content.get("target").and_then(|t| t.get("review_id")))
        .filter(|v| !v.is_empty() && v.len() <= 128)
        .ok_or("feedback names no review block")?;
    let request = text(content.get("request")).ok_or("feedback has no request")?;
    if request.trim().is_empty() || request.len() > MAX_FEEDBACK_REQUEST_BYTES {
        return Err("feedback request must be nonblank and bounded");
    }
    if single_tag(event, "target")? != block
        || single_tag(event, "target_revision")? != revision
        || single_tag(event, "synaxis_artifact")? != synaxis_artifact_id
        || single_tag(event, "payload")? != payload_digest
    {
        return Err("feedback tags disagree with its signed content");
    }
    Ok(FeedbackTarget {
        review_artifact,
        revision: event_id(revision)?,
        synaxis_artifact_id: synaxis_artifact_id.to_owned(),
        payload_digest: payload_digest.to_owned(),
    })
}

/// Whether a `synaxis.html-review/v1` revision's content carries the Synaxis
/// artifact ID and payload digest a feedback create claims, with the
/// presentation hash agreeing with the digest.
pub fn review_binds_feedback(review_content: &str, target: &FeedbackTarget) -> bool {
    let Ok(content) = serde_json::from_str::<serde_json::Value>(review_content) else {
        return false;
    };
    let field = |path: [&str; 2]| {
        content
            .get(path[0])
            .and_then(|v| v.get(path[1]))
            .and_then(|v| v.as_str())
    };
    field(["artifact", "id"]) == Some(target.synaxis_artifact_id.as_str())
        && field(["artifact", "payload_digest"]) == Some(target.payload_digest.as_str())
        && field(["presentation", "blob_sha256"]) == Some(target.payload_digest.as_str())
}

/// Relay rejection for an event whose `created_at` is outside the ingest
/// freshness window. Native clients recognise a refused review-feedback wake by
/// this exact text, so ingest and clients must share this constant.
pub const STALE_EVENT_TIMESTAMP_REJECTION: &str =
    "invalid: event timestamp too far from server time";

/// The bindings of a validated review-feedback wake: a kind-9 reply carrying a
/// `feedback` tag and an `artifact` tag that wakes the executive agent. The
/// relay records these per `(author, feedback revision)` so a retry with a
/// fresh timestamp is acknowledged instead of waking the agent twice.
#[derive(Debug, PartialEq, Eq)]
pub struct FeedbackWake {
    /// Stable NIP-AR UUID of the `synaxis.artifact-feedback` artifact.
    pub feedback_artifact: Uuid,
    /// Event ID of the feedback revision (32 bytes).
    pub feedback_event: Vec<u8>,
    /// Stable NIP-AR UUID of the reviewed artifact.
    pub reviewed_artifact: Uuid,
    /// Event ID of the reviewed revision (32 bytes).
    pub reviewed_event: Vec<u8>,
    /// Executive agent pubkey the wake mentions (32 bytes).
    pub agent: Vec<u8>,
    /// Thread root event ID: the `root`-marked reference when present, else the
    /// parent (32 bytes).
    pub root_event: Vec<u8>,
    /// Direct parent event ID: the `reply`-marked reference (32 bytes).
    pub parent_event: Vec<u8>,
}

/// Whether `event` claims to be a review-feedback wake: a kind-9 message with
/// at least one `feedback` tag. Other kind-9 messages, including the factory's
/// review-ready notification (which carries only an `artifact` tag), are not
/// candidates and keep their ordinary ingest path.
pub fn is_feedback_wake_candidate(event: &Event) -> bool {
    u32::from(event.kind.as_u16()) == crate::kind::KIND_STREAM_MESSAGE
        && event.tags.iter().any(|t| {
            t.as_slice()
                .first()
                .is_some_and(|n| n.as_str() == "feedback")
        })
}

/// Every tag of `event` whose name is `name`.
fn tags_named<'a>(event: &'a Event, name: &'a str) -> impl Iterator<Item = &'a [String]> + 'a {
    event
        .tags
        .iter()
        .map(|t| t.as_slice())
        .filter(move |t| t.first().is_some_and(|n| n.as_str() == name))
}

/// The single `[name, <canonical UUID>, <event ID>]` tag of a wake.
fn wake_revision_ref(
    event: &Event,
    name: &str,
    refusal: &'static str,
) -> Result<(Uuid, Vec<u8>), &'static str> {
    let mut tags = tags_named(event, name);
    match (tags.next(), tags.next()) {
        (Some([_, artifact, revision]), None) => Ok((
            canonical_uuid(artifact).map_err(|_| refusal)?,
            event_id(revision).map_err(|_| refusal)?,
        )),
        _ => Err(refusal),
    }
}

/// Validate a review-feedback wake. The tag layout is exactly what the desktop
/// emits: one `h`; NIP-10 `e` tags (`reply`, plus `root` only when the parent
/// is not itself the root); one `p` naming the executive agent; one `feedback`
/// tag and one `artifact` tag, each `[name, <canonical UUID>, <event ID>]`.
/// Whether the named revisions exist is the relay's database check, not this
/// function's.
pub fn validate_feedback_wake(event: &Event) -> Result<FeedbackWake, &'static str> {
    if !is_feedback_wake_candidate(event) {
        return Err("not a review-feedback wake");
    }
    if event.content.trim().is_empty() {
        return Err("feedback wake content must be nonblank");
    }
    let (feedback_artifact, feedback_event) = wake_revision_ref(
        event,
        "feedback",
        "feedback wake needs exactly one [feedback, UUID, event ID] tag",
    )?;
    let (reviewed_artifact, reviewed_event) = wake_revision_ref(
        event,
        "artifact",
        "feedback wake needs exactly one [artifact, UUID, event ID] tag",
    )?;
    let mut home = tags_named(event, "h");
    if !matches!((home.next(), home.next()), (Some([_, _]), None)) {
        return Err("feedback wake needs exactly one two-element h tag");
    }
    let mut agents = tags_named(event, "p");
    let agent = match (agents.next(), agents.next()) {
        (Some([_, agent]), None) => {
            event_id(agent).map_err(|_| "feedback wake p tag must be a hex pubkey")?
        }
        _ => return Err("feedback wake needs exactly one p tag naming the agent"),
    };
    let mut root = None;
    let mut reply = None;
    for tag in tags_named(event, "e") {
        let slot = match tag {
            [_, _, _, marker] if marker == "reply" => &mut reply,
            [_, _, _, marker] if marker == "root" => &mut root,
            _ => return Err("feedback wake e tags must be marked reply or root"),
        };
        let id = event_id(&tag[1])
            .map_err(|_| "feedback wake e tags must carry 64-character hex event IDs")?;
        if slot.replace(id).is_some() {
            return Err("feedback wake e tags must each occur at most once");
        }
    }
    let parent_event = reply.ok_or("feedback wake needs a reply-marked e tag")?;
    Ok(FeedbackWake {
        feedback_artifact,
        feedback_event,
        reviewed_artifact,
        reviewed_event,
        agent,
        root_event: root.unwrap_or_else(|| parent_event.clone()),
        parent_event,
    })
}

#[cfg(test)]
mod feedback_tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    const D: &str = "24737c81-e5e8-4412-bb47-f446813cfeba";
    const HOME: &str = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
    const REVIEW: &str = "04737c81-e5e8-4412-bb47-f446813cfeba";
    const REV: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const DIGEST: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    fn content() -> serde_json::Value {
        serde_json::json!({
            "schema": "synaxis.artifact-feedback/v1",
            "reviewed": {
                "buzz_artifact_id": REVIEW,
                "buzz_revision_event_id": REV,
                "synaxis_artifact_id": "01SYN",
                "payload_digest": DIGEST,
            },
            "target": { "review_id": "checkout.primary-action", "title": "Primary" },
            "request": "Move this up.",
        })
    }

    fn feedback(
        op: &str,
        content: &serde_json::Value,
        tweak: impl Fn(&mut Vec<Vec<String>>),
    ) -> Event {
        let mut tags: Vec<Vec<String>> = [
            vec!["ar", "1"],
            vec!["d", D],
            vec!["h", HOME],
            vec!["type", SYNAXIS_FEEDBACK_TYPE],
            vec!["title", "Feedback"],
            vec!["op", op],
            vec!["target", "checkout.primary-action"],
            vec!["target_revision", REV],
            vec!["synaxis_artifact", "01SYN"],
            vec!["payload", DIGEST],
        ]
        .into_iter()
        .map(|t| t.into_iter().map(str::to_owned).collect())
        .collect();
        if op != "create" {
            tags.push(vec!["prev".into(), REV.into()]);
        }
        tweak(&mut tags);
        EventBuilder::new(Kind::Custom(45010), content.to_string())
            .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }

    fn check(event: &Event) -> Result<FeedbackTarget, &'static str> {
        validate_feedback_create(event, &validate(event).unwrap())
    }

    #[test]
    fn a_well_formed_create_names_its_exact_target() {
        let target = check(&feedback("create", &content(), |_| {})).unwrap();
        assert_eq!(target.review_artifact, Uuid::parse_str(REVIEW).unwrap());
        assert_eq!(target.revision, hex::decode(REV).unwrap());
        assert_eq!(target.synaxis_artifact_id, "01SYN");
        assert_eq!(target.payload_digest, DIGEST);
    }

    #[test]
    fn feedback_is_create_only() {
        for op in ["update", "restore"] {
            let event = feedback(op, &content(), |_| {});
            assert_eq!(
                check(&event).unwrap_err(),
                "synaxis feedback is create-only",
                "{op}"
            );
        }
        let delete = EventBuilder::new(Kind::Custom(45010), "")
            .tags(
                [
                    vec!["ar", "1"],
                    vec!["d", D],
                    vec!["h", HOME],
                    vec!["type", SYNAXIS_FEEDBACK_TYPE],
                    vec!["op", "delete"],
                    vec!["prev", REV],
                ]
                .into_iter()
                .map(|t| Tag::parse(t).unwrap()),
            )
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert_eq!(
            check(&delete).unwrap_err(),
            "synaxis feedback is create-only"
        );
    }

    #[test]
    fn tags_must_restate_the_signed_content() {
        for name in ["target", "target_revision", "synaxis_artifact", "payload"] {
            let swapped = feedback("create", &content(), |tags| {
                for tag in tags.iter_mut().filter(|t| t[0] == name) {
                    tag[1] = if name == "target_revision" || name == "payload" {
                        "3".repeat(64)
                    } else {
                        "other".into()
                    };
                }
            });
            assert!(check(&swapped).is_err(), "{name}");
            let missing = feedback("create", &content(), |tags| tags.retain(|t| t[0] != name));
            assert!(check(&missing).is_err(), "{name} missing");
            let doubled = feedback("create", &content(), |tags| {
                let copy = tags.iter().find(|t| t[0] == name).cloned().unwrap();
                tags.push(copy);
            });
            assert!(check(&doubled).is_err(), "{name} duplicated");
        }
    }

    #[test]
    fn malformed_content_is_refused() {
        let mut cases = Vec::new();
        let mut wrong_schema = content();
        wrong_schema["schema"] = serde_json::json!("synaxis.artifact-feedback/v2");
        cases.push(wrong_schema);
        for field in [
            "buzz_artifact_id",
            "buzz_revision_event_id",
            "synaxis_artifact_id",
            "payload_digest",
        ] {
            let mut bad = content();
            bad["reviewed"][field] = serde_json::json!("nope");
            cases.push(bad);
            let mut missing = content();
            missing["reviewed"].as_object_mut().unwrap().remove(field);
            cases.push(missing);
        }
        let mut blank = content();
        blank["request"] = serde_json::json!("   ");
        cases.push(blank);
        let mut huge = content();
        huge["request"] = serde_json::json!("x".repeat(MAX_FEEDBACK_REQUEST_BYTES + 1));
        cases.push(huge);
        for c in cases {
            assert!(check(&feedback("create", &c, |_| {})).is_err(), "{c}");
        }
    }

    #[test]
    fn a_review_binds_only_its_own_artifact_and_digest() {
        let target = check(&feedback("create", &content(), |_| {})).unwrap();
        let review = |id: &str, digest: &str, blob: &str| {
            serde_json::json!({
                "artifact": { "id": id, "payload_digest": digest },
                "presentation": { "blob_sha256": blob },
            })
            .to_string()
        };
        assert!(review_binds_feedback(
            &review("01SYN", DIGEST, DIGEST),
            &target
        ));
        assert!(!review_binds_feedback(
            &review("01OTHER", DIGEST, DIGEST),
            &target
        ));
        assert!(!review_binds_feedback(
            &review("01SYN", &"4".repeat(64), &"4".repeat(64)),
            &target
        ));
        assert!(!review_binds_feedback(
            &review("01SYN", DIGEST, &"4".repeat(64)),
            &target
        ));
        assert!(!review_binds_feedback("not json", &target));
    }
}

#[cfg(test)]
mod wake_tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    const CHANNEL: &str = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
    const FEEDBACK: &str = "24737c81-e5e8-4412-bb47-f446813cfeba";
    const REVIEW: &str = "04737c81-e5e8-4412-bb47-f446813cfeba";
    const FEEDBACK_REV: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const REVIEW_REV: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const ROOT: &str = "3333333333333333333333333333333333333333333333333333333333333333";
    const PARENT: &str = "4444444444444444444444444444444444444444444444444444444444444444";
    const AGENT: &str = "5555555555555555555555555555555555555555555555555555555555555555";

    // Positions in `depth_two()`.
    const H: usize = 0;
    const ROOT_E: usize = 1;
    const REPLY_E: usize = 2;
    const P: usize = 3;
    const FEEDBACK_T: usize = 4;
    const ARTIFACT_T: usize = 5;

    type Tags = Vec<Vec<String>>;

    fn tag(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| (*p).to_owned()).collect()
    }

    /// The desktop's layout for a reply two levels deep: root and reply `e` tags.
    fn depth_two() -> Tags {
        vec![
            tag(&["h", CHANNEL]),
            tag(&["e", ROOT, "", "root"]),
            tag(&["e", PARENT, "", "reply"]),
            tag(&["p", AGENT]),
            tag(&["feedback", FEEDBACK, FEEDBACK_REV]),
            tag(&["artifact", REVIEW, REVIEW_REV]),
        ]
    }

    /// A reply to the thread root carries only the reply-marked `e` tag.
    fn depth_one() -> Tags {
        let mut tags = depth_two();
        tags.remove(ROOT_E);
        tags
    }

    fn with_tag(index: usize, parts: &[&str]) -> Tags {
        let mut tags = depth_two();
        tags[index] = tag(parts);
        tags
    }

    fn message(kind: u16, content: &str, tags: Tags) -> Event {
        EventBuilder::new(Kind::Custom(kind), content)
            .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }

    fn wake_event(tags: Tags) -> Event {
        message(9, "Please address the feedback.", tags)
    }

    fn refused(tags: Tags) -> bool {
        validate_feedback_wake(&wake_event(tags)).is_err()
    }

    fn id(value: &str) -> Vec<u8> {
        hex::decode(value).unwrap()
    }

    #[test]
    fn a_depth_one_wake_is_rooted_at_its_reply() {
        let wake = validate_feedback_wake(&wake_event(depth_one())).unwrap();
        assert_eq!(
            wake,
            FeedbackWake {
                feedback_artifact: Uuid::parse_str(FEEDBACK).unwrap(),
                feedback_event: id(FEEDBACK_REV),
                reviewed_artifact: Uuid::parse_str(REVIEW).unwrap(),
                reviewed_event: id(REVIEW_REV),
                agent: id(AGENT),
                root_event: id(PARENT),
                parent_event: id(PARENT),
            }
        );
    }

    #[test]
    fn a_depth_two_wake_names_its_root_and_parent() {
        let wake = validate_feedback_wake(&wake_event(depth_two())).unwrap();
        assert_eq!(wake.root_event, id(ROOT));
        assert_eq!(wake.parent_event, id(PARENT));
        assert_eq!(wake.feedback_event, id(FEEDBACK_REV));
        assert_eq!(wake.reviewed_event, id(REVIEW_REV));
    }

    #[test]
    fn unrelated_tags_and_relay_hints_do_not_change_a_wake() {
        let mut tags = depth_two();
        tags.push(tag(&["client", "buzz-desktop"]));
        tags[REPLY_E][2] = "wss://relay.example".into();
        assert_eq!(
            validate_feedback_wake(&wake_event(tags)),
            validate_feedback_wake(&wake_event(depth_two()))
        );
    }

    #[test]
    fn only_kind_nine_messages_with_a_feedback_tag_are_candidates() {
        assert!(is_feedback_wake_candidate(&wake_event(depth_two())));
        // A malformed feedback tag still makes the message a candidate, so it is
        // validated and refused rather than waved through.
        let malformed = wake_event(vec![tag(&["feedback"])]);
        assert!(is_feedback_wake_candidate(&malformed));
        assert!(validate_feedback_wake(&malformed).is_err());

        // The factory's review-ready notification carries an `artifact` tag but
        // no `feedback` tag and keeps its ordinary path.
        let mut ready = depth_two();
        ready.remove(FEEDBACK_T);
        let ready = wake_event(ready);
        assert!(!is_feedback_wake_candidate(&ready));
        assert_eq!(
            validate_feedback_wake(&ready),
            Err("not a review-feedback wake")
        );

        let plain = message(9, "hello", vec![tag(&["h", CHANNEL])]);
        assert!(!is_feedback_wake_candidate(&plain));
        for kind in [1, 40002, 45010] {
            let other = message(kind, "{}", depth_two());
            assert!(!is_feedback_wake_candidate(&other), "kind {kind}");
            assert!(validate_feedback_wake(&other).is_err(), "kind {kind}");
        }
    }

    #[test]
    fn every_required_tag_must_be_present() {
        for index in [H, REPLY_E, P, FEEDBACK_T, ARTIFACT_T] {
            let mut tags = depth_two();
            let removed = tags.remove(index);
            assert!(refused(tags), "{:?} missing", removed[0]);
        }
        // A root-marked reference alone does not name the direct parent.
        let mut root_only = depth_two();
        root_only.remove(REPLY_E);
        assert!(refused(root_only));
    }

    #[test]
    fn no_tag_may_be_duplicated() {
        for index in [H, ROOT_E, REPLY_E, P, FEEDBACK_T, ARTIFACT_T] {
            let mut exact = depth_two();
            exact.push(exact[index].clone());
            assert!(refused(exact), "{:?} repeated", depth_two()[index]);

            let mut other = depth_two();
            let mut changed = other[index].clone();
            changed[1] = match index {
                H => "6a1657ac-f7aa-5db0-b632-d8bbeb6dfb50".to_owned(),
                FEEDBACK_T | ARTIFACT_T => "34737c81-e5e8-4412-bb47-f446813cfeba".to_owned(),
                _ => "6".repeat(64),
            };
            other.push(changed);
            assert!(
                refused(other),
                "{:?} repeated with another value",
                depth_two()[index]
            );
        }
    }

    #[test]
    fn a_third_or_unmarked_e_tag_is_refused() {
        let mention = tag(&["e", ROOT, "", "mention"]);
        let unmarked = tag(&["e", ROOT]);
        let unmarked_hinted = tag(&["e", ROOT, "wss://relay.example"]);
        for extra in [mention, unmarked, unmarked_hinted] {
            let mut with_extra = depth_two();
            with_extra.push(extra.clone());
            assert!(refused(with_extra), "{extra:?}");
        }
    }

    #[test]
    fn malformed_tags_are_refused() {
        fn assert_refused(label: &str, index: usize, parts: &[&str]) {
            assert!(refused(with_tag(index, parts)), "{label}");
        }
        let upper_id = "A".repeat(64);
        assert_refused("blank h", H, &["h"]);
        assert_refused("h with a relay hint", H, &["h", CHANNEL, "x"]);
        assert_refused("p without a pubkey", P, &["p"]);
        assert_refused("p with a short pubkey", P, &["p", "abcd"]);
        assert_refused("p with an uppercase pubkey", P, &["p", &upper_id]);
        assert_refused("p with a petname", P, &["p", AGENT, "", "name"]);
        assert_refused("reply without a marker", REPLY_E, &["e", PARENT]);
        assert_refused(
            "reply with an empty marker",
            REPLY_E,
            &["e", PARENT, "", ""],
        );
        assert_refused(
            "reply with a pubkey",
            REPLY_E,
            &["e", PARENT, "", "reply", AGENT],
        );
        assert_refused(
            "reply with a short id",
            REPLY_E,
            &["e", "abcd", "", "reply"],
        );
        assert_refused(
            "reply with an uppercase id",
            REPLY_E,
            &["e", &upper_id, "", "reply"],
        );
        assert_refused("root with a short id", ROOT_E, &["e", "abcd", "", "root"]);
        assert_refused(
            "root with an uppercase id",
            ROOT_E,
            &["e", &upper_id, "", "root"],
        );
        assert_refused("root marked mention", ROOT_E, &["e", ROOT, "", "mention"]);

        let nil = "00000000-0000-0000-0000-000000000000";
        let simple_uuid = FEEDBACK.replace('-', "");
        for (name, index, uuid, rev) in [
            ("feedback", FEEDBACK_T, FEEDBACK, FEEDBACK_REV),
            ("artifact", ARTIFACT_T, REVIEW, REVIEW_REV),
        ] {
            let upper_uuid = uuid.to_uppercase();
            assert_refused("no revision", index, &[name, uuid]);
            assert_refused("fourth element", index, &[name, uuid, rev, "x"]);
            assert_refused("uppercase UUID", index, &[name, &upper_uuid, rev]);
            assert_refused("nil UUID", index, &[name, nil, rev]);
            assert_refused("simple UUID", index, &[name, &simple_uuid, rev]);
            assert_refused("non-UUID", index, &[name, "not-a-uuid", rev]);
            assert_refused("short revision", index, &[name, uuid, "abcd"]);
            assert_refused("uppercase revision", index, &[name, uuid, &upper_id]);
        }
    }

    #[test]
    fn a_wake_needs_nonblank_content() {
        for content in ["", "   ", "\n\t"] {
            let event = message(9, content, depth_two());
            assert_eq!(
                validate_feedback_wake(&event),
                Err("feedback wake content must be nonblank"),
                "{content:?}"
            );
        }
    }
}
