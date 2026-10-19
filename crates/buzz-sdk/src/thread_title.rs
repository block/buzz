//! Shared, editable thread titles stored as NIP-AR artifacts (kind 45010).
//!
//! Convention (client-defined; the relay treats it as opaque):
//! - `type` = [`THREAD_TITLE_TYPE`]
//! - `d` = UUIDv5([`THREAD_TITLE_NAMESPACE`], `"<type>:<lowercase root hex>"`),
//!   so every client derives the same identity and a thread has at most one title.
//!   The type is part of the name because NIP-AR never reuses a deleted `d`:
//!   moving to another type name must yield fresh ids.
//! - `h` = channel, `root` = thread root, `title` = the title
//! - `content` = the title again, so a reader that ignores tags still sees it.
//!   This does not make titles searchable: relay search does not index kind
//!   45010, and client search does not query it.
//!
//! The relay enforces permissions (same as posting in the channel) and
//! compare-and-swap ordering through `prev`; clearing is a soft `delete`, and
//! setting a cleared title is a `restore`.

use std::collections::HashSet;

use nostr::{Event, EventBuilder, Kind, Tag};
use uuid::Uuid;

use crate::SdkError;

/// Artifact type for thread titles, as proposed upstream in block/buzz#6739.
/// NIP-AR reserves `buzz.*` for published Buzz client contracts; this one is
/// not published yet, and the relay does not enforce the reservation.
pub const THREAD_TITLE_TYPE: &str = "buzz.thread-title";

/// Namespace for deriving a thread title's artifact id from its type and root event id.
pub const THREAD_TITLE_NAMESPACE: Uuid = uuid::uuid!("2039f9bf-c754-483e-a7cf-56f1ec0e4abe");

/// Maximum title size in UTF-8 bytes (the NIP-AR `title` limit).
pub const MAX_THREAD_TITLE_BYTES: usize = 512;

/// Writes that lose a compare-and-swap race re-read the head and retry this many times in total.
pub const MAX_THREAD_TITLE_WRITE_ATTEMPTS: usize = 3;

const KIND_ARTIFACT: u16 = buzz_core::kind::KIND_ARTIFACT as u16;

/// Derive the artifact id (`d`) for a thread root.
pub fn thread_title_id(root_event_id: &str) -> Uuid {
    let name = format!("{THREAD_TITLE_TYPE}:{}", root_event_id.to_ascii_lowercase());
    Uuid::new_v5(&THREAD_TITLE_NAMESPACE, name.as_bytes())
}

/// Trim and bound a title; blank titles are rejected (clearing is a separate operation).
pub fn normalize_thread_title(title: &str) -> Result<String, SdkError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(SdkError::InvalidInput("title must not be blank".into()));
    }
    if title.len() > MAX_THREAD_TITLE_BYTES {
        return Err(SdkError::ContentTooLarge {
            max: MAX_THREAD_TITLE_BYTES,
            got: title.len(),
        });
    }
    Ok(title.to_string())
}

/// Whether a relay rejection is a lost compare-and-swap race worth re-reading and retrying.
pub fn is_thread_title_head_conflict(message: &str) -> bool {
    message.contains("artifact head changed") || message.contains("artifact identity is taken")
}

/// HTTP `/query` filter for one thread's live title (deleted titles are omitted).
pub fn thread_title_current_filter(root_event_id: &str) -> serde_json::Value {
    serde_json::json!({
        "artifact": "current",
        "#d": [thread_title_id(root_event_id).to_string()],
        "limit": 1,
    })
}

/// HTTP `/query` filter for every revision of one thread's title, including deletions.
pub fn thread_title_history_filter(root_event_id: &str) -> serde_json::Value {
    serde_json::json!({
        "artifact": "history",
        "#d": [thread_title_id(root_event_id).to_string()],
        "limit": 1000,
    })
}

/// HTTP `/query` filter for live titles, newest revision first, optionally in one channel.
pub fn thread_title_list_filter(channel: Option<Uuid>, limit: u32) -> serde_json::Value {
    let mut filter = serde_json::json!({
        "artifact": "current",
        "#type": [THREAD_TITLE_TYPE],
        "limit": limit,
    });
    if let Some(channel) = channel {
        filter["#h"] = serde_json::json!([channel.to_string()]);
    }
    filter
}

/// Most roots one [`thread_titles_for_roots_filter`] should name; the relay
/// caps an artifact query at 256 predicate values.
pub const MAX_THREAD_TITLE_ROOTS_PER_QUERY: usize = 100;

/// HTTP `/query` filter for the live titles of specific thread roots. Looks
/// titles up by their derived `d`, so it never depends on how many other
/// titles exist.
pub fn thread_titles_for_roots_filter(root_event_ids: &[&str]) -> serde_json::Value {
    let ids: Vec<String> = root_event_ids
        .iter()
        .map(|root| thread_title_id(root).to_string())
        .collect();
    serde_json::json!({
        "artifact": "current",
        "#type": [THREAD_TITLE_TYPE],
        "#d": ids,
        "limit": ids.len().max(1),
    })
}

fn tag_value<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|tag| match tag.as_slice() {
        [key, value, ..] if key == name => Some(value.as_str()),
        _ => None,
    })
}

/// A live thread title as read from its current revision.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ThreadTitle {
    /// The title text.
    pub title: String,
    /// Home channel UUID.
    pub channel: String,
    /// Thread root event id.
    pub root: String,
    /// Artifact id (`d`).
    pub artifact: String,
    /// Current revision event id.
    pub revision: String,
    /// Pubkey of whoever last set the title.
    pub author: String,
    /// Unix seconds of the current revision.
    pub updated_at: u64,
}

/// Read a thread title from a current revision; `None` for other types, deletions, or malformed events.
pub fn parse_thread_title(event: &Event) -> Option<ThreadTitle> {
    if event.kind.as_u16() != KIND_ARTIFACT
        || tag_value(event, "type") != Some(THREAD_TITLE_TYPE)
        || tag_value(event, "op") == Some("delete")
    {
        return None;
    }
    Some(ThreadTitle {
        title: tag_value(event, "title")?.to_string(),
        channel: tag_value(event, "h")?.to_string(),
        root: tag_value(event, "root")?.to_string(),
        artifact: tag_value(event, "d")?.to_string(),
        revision: event.id.to_hex(),
        author: event.pubkey.to_hex(),
        updated_at: event.created_at.as_secs(),
    })
}

/// The artifact's current accepted revision, as seen by this reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadTitleHead {
    /// Revision event id, used as the next write's `prev`.
    pub event_id: String,
    /// Whether the head is a deletion (only `restore` may follow).
    pub deleted: bool,
    /// Title on the head, absent on deletions.
    pub title: Option<String>,
}

impl ThreadTitleHead {
    /// The head represented by one revision.
    pub fn from_event(event: &Event) -> Self {
        Self {
            event_id: event.id.to_hex(),
            deleted: tag_value(event, "op") == Some("delete"),
            title: tag_value(event, "title").map(str::to_string),
        }
    }
}

/// Pick the head of a revision history: the revision no other revision names
/// in `prev`. Redaction can hide a link and leave several candidates; the
/// newest wins, and the relay's `prev` check rejects the write if it is stale.
pub fn select_thread_title_head(revisions: &[Event]) -> Option<ThreadTitleHead> {
    let superseded: HashSet<&str> = revisions
        .iter()
        .filter_map(|e| tag_value(e, "prev"))
        .collect();
    revisions
        .iter()
        .filter(|e| !superseded.contains(e.id.to_hex().as_str()))
        .max_by_key(|e| e.created_at)
        .map(ThreadTitleHead::from_event)
}

/// What a write must do, given the current head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadTitlePlan {
    /// No artifact yet.
    Create,
    /// Rename a live title.
    Update {
        /// Current head.
        prev: String,
    },
    /// Set a title that was cleared.
    Restore {
        /// Current (deletion) head.
        prev: String,
    },
    /// Clear a live title.
    Delete {
        /// Current head.
        prev: String,
    },
    /// Already in the requested state.
    Unchanged,
}

impl ThreadTitlePlan {
    /// The NIP-AR `op` for this plan.
    pub fn op(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update { .. } => "update",
            Self::Restore { .. } => "restore",
            Self::Delete { .. } => "delete",
            Self::Unchanged => "unchanged",
        }
    }
}

/// Plan a write; `desired = None` clears the title.
pub fn plan_thread_title(head: Option<&ThreadTitleHead>, desired: Option<&str>) -> ThreadTitlePlan {
    match (head, desired) {
        (None, Some(_)) => ThreadTitlePlan::Create,
        (None, None) => ThreadTitlePlan::Unchanged,
        (Some(h), None) if h.deleted => ThreadTitlePlan::Unchanged,
        (Some(h), None) => ThreadTitlePlan::Delete {
            prev: h.event_id.clone(),
        },
        (Some(h), Some(_)) if h.deleted => ThreadTitlePlan::Restore {
            prev: h.event_id.clone(),
        },
        (Some(h), Some(t)) if h.title.as_deref() == Some(t) => ThreadTitlePlan::Unchanged,
        (Some(h), Some(_)) => ThreadTitlePlan::Update {
            prev: h.event_id.clone(),
        },
    }
}

/// Build the revision for `plan`. Returns `None` for [`ThreadTitlePlan::Unchanged`].
pub fn build_thread_title_revision(
    channel: Uuid,
    root_event_id: &str,
    plan: &ThreadTitlePlan,
    title: Option<&str>,
) -> Result<Option<EventBuilder>, SdkError> {
    let prev = match plan {
        ThreadTitlePlan::Unchanged => return Ok(None),
        ThreadTitlePlan::Create => None,
        ThreadTitlePlan::Update { prev }
        | ThreadTitlePlan::Restore { prev }
        | ThreadTitlePlan::Delete { prev } => Some(prev.as_str()),
    };
    let root = root_event_id.to_ascii_lowercase();
    let id = thread_title_id(&root).to_string();
    let channel = channel.to_string();
    let mut parts: Vec<[&str; 2]> = vec![
        ["ar", "1"],
        ["d", &id],
        ["h", &channel],
        ["type", THREAD_TITLE_TYPE],
        ["op", plan.op()],
        ["root", &root],
    ];
    if let Some(prev) = prev {
        parts.push(["prev", prev]);
    }
    let content = if matches!(plan, ThreadTitlePlan::Delete { .. }) {
        ""
    } else {
        let title = title
            .ok_or_else(|| SdkError::InvalidInput("title required for this operation".into()))?;
        parts.push(["title", title]);
        title
    };
    let tags = parts
        .into_iter()
        .map(|t| Tag::parse(t).map_err(|e| SdkError::InvalidTag(e.to_string())))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(
        EventBuilder::new(Kind::Custom(KIND_ARTIFACT), content).tags(tags),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{Keys, Timestamp};

    const ROOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn revision(
        keys: &Keys,
        created_at: u64,
        plan: &ThreadTitlePlan,
        title: Option<&str>,
    ) -> Event {
        build_thread_title_revision(Uuid::new_v4(), ROOT, plan, title)
            .unwrap()
            .unwrap()
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(keys)
            .unwrap()
    }

    #[test]
    fn id_is_uuidv5_of_type_and_root_hex() {
        // Independently computed with Python's uuid.uuid5(namespace, "buzz.thread-title:" + root).
        assert_eq!(
            thread_title_id(ROOT).to_string(),
            "0d943738-f875-5089-91fe-135fe16d0f72"
        );
        assert_eq!(thread_title_id(&ROOT.to_uppercase()), thread_title_id(ROOT));
    }

    #[test]
    fn head_follows_prev_chain_not_timestamps() {
        let keys = Keys::generate();
        let a = revision(&keys, 100, &ThreadTitlePlan::Create, Some("one"));
        let b = revision(
            &keys,
            300,
            &ThreadTitlePlan::Update {
                prev: a.id.to_hex(),
            },
            Some("two"),
        );
        // Older timestamp, but it supersedes `b`.
        let c = revision(
            &keys,
            200,
            &ThreadTitlePlan::Delete {
                prev: b.id.to_hex(),
            },
            None,
        );
        let head = select_thread_title_head(&[a, b, c.clone()]).unwrap();
        assert_eq!(head.event_id, c.id.to_hex());
        assert!(head.deleted);
        assert_eq!(select_thread_title_head(&[]), None);
    }

    #[test]
    fn plan_covers_lifecycle() {
        use ThreadTitlePlan::*;
        let live = ThreadTitleHead {
            event_id: "h".into(),
            deleted: false,
            title: Some("x".into()),
        };
        let gone = ThreadTitleHead {
            event_id: "h".into(),
            deleted: true,
            title: None,
        };
        let prev = || "h".to_string();
        assert_eq!(plan_thread_title(None, Some("x")), Create);
        assert_eq!(plan_thread_title(None, None), Unchanged);
        assert_eq!(plan_thread_title(Some(&live), Some("x")), Unchanged);
        assert_eq!(
            plan_thread_title(Some(&live), Some("y")),
            Update { prev: prev() }
        );
        assert_eq!(
            plan_thread_title(Some(&live), None),
            Delete { prev: prev() }
        );
        assert_eq!(
            plan_thread_title(Some(&gone), Some("y")),
            Restore { prev: prev() }
        );
        assert_eq!(plan_thread_title(Some(&gone), None), Unchanged);
    }

    /// Every op emitted must pass the relay's own envelope validator.
    #[test]
    fn revisions_pass_relay_validation() {
        let channel = Uuid::new_v4();
        let prev = "b".repeat(64);
        for (plan, title) in [
            (ThreadTitlePlan::Create, Some("Release checklist")),
            (
                ThreadTitlePlan::Update { prev: prev.clone() },
                Some("Renamed"),
            ),
            (
                ThreadTitlePlan::Restore { prev: prev.clone() },
                Some("Back"),
            ),
            (ThreadTitlePlan::Delete { prev: prev.clone() }, None),
        ] {
            let event = build_thread_title_revision(channel, &ROOT.to_uppercase(), &plan, title)
                .unwrap()
                .unwrap()
                .sign_with_keys(&Keys::generate())
                .unwrap();
            let env =
                buzz_core::artifact::validate(&event).unwrap_or_else(|e| panic!("{plan:?}: {e}"));
            assert_eq!(env.id, thread_title_id(ROOT));
            assert_eq!(env.home, channel);
            assert_eq!(env.artifact_type, THREAD_TITLE_TYPE);
            assert_eq!(env.root.as_deref(), Some([0xaa; 32].as_slice()));
            assert_eq!(event.content, title.unwrap_or(""));
            assert_eq!(
                parse_thread_title(&event).map(|t| t.title),
                title.map(str::to_string)
            );
        }
        assert!(
            build_thread_title_revision(channel, ROOT, &ThreadTitlePlan::Unchanged, Some("x"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn parse_ignores_other_artifact_types() {
        let event = EventBuilder::new(Kind::Custom(KIND_ARTIFACT), "x")
            .tags([
                Tag::parse(["type", "buzz.task"]).unwrap(),
                Tag::parse(["title", "x"]).unwrap(),
            ])
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert_eq!(parse_thread_title(&event), None);
    }

    #[test]
    fn title_normalization() {
        assert_eq!(normalize_thread_title("  Spaced  ").unwrap(), "Spaced");
        assert!(normalize_thread_title("   ").is_err());
        assert!(normalize_thread_title(&"é".repeat(256)).is_ok()); // 512 bytes
        assert!(normalize_thread_title(&"é".repeat(257)).is_err());
    }

    #[test]
    fn roots_filter_names_each_derived_id() {
        let other = "b".repeat(64);
        let filter = thread_titles_for_roots_filter(&[ROOT, &other]);
        assert_eq!(filter["artifact"], "current");
        assert_eq!(filter["#type"], serde_json::json!([THREAD_TITLE_TYPE]));
        assert_eq!(
            filter["#d"],
            serde_json::json!([
                thread_title_id(ROOT).to_string(),
                thread_title_id(&other).to_string()
            ])
        );
        assert_eq!(filter["limit"], 2);
        // A limit of 0 is rejected by the relay.
        assert_eq!(thread_titles_for_roots_filter(&[])["limit"], 1);
    }

    #[test]
    fn only_cas_losses_are_retryable() {
        assert!(is_thread_title_head_conflict(
            "conflict: artifact head changed"
        ));
        assert!(is_thread_title_head_conflict(
            "conflict: artifact identity is taken"
        ));
        assert!(!is_thread_title_head_conflict(
            "conflict: artifact home changed"
        ));
        assert!(!is_thread_title_head_conflict(
            "invalid: delete preserves root"
        ));
    }
}
