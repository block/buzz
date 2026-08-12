//! NIP-29 discovery-event tags derived from channel rows.
//!
//! kind:39000 is what every client reads to decide whether a channel is
//! archived, ephemeral, or has a topic — so any writer that builds those tags
//! by hand becomes a second source of truth. When the two disagree the loser is
//! silent: a re-emit that forgets `archived` un-hides a channel in every
//! sidebar while the database still says it is archived, and nothing logs.
//!
//! Tags are returned as raw string arrays so this stays a pure function of the
//! rows — callers own the `Tag::parse` and the signing.

use uuid::Uuid;

use crate::channel::{ChannelRecord, MemberRecord};

/// kind:39000 (NIP-29 group metadata) tags for `channel`.
///
/// `members` is only read for DM channels, whose participants are inlined so
/// clients can resolve display names without a second kind:39002 fetch.
pub fn nip29_metadata_tags(channel: &ChannelRecord, members: &[MemberRecord]) -> Vec<Vec<String>> {
    let group_id = channel.id.to_string();
    let mut tags: Vec<Vec<String>> = vec![vec!["d".into(), group_id]];

    tags.push(vec!["name".into(), channel.name.clone()]);
    if let Some(desc) = channel.description.as_ref().filter(|d| !d.is_empty()) {
        tags.push(vec!["about".into(), desc.clone()]);
    }
    if channel.visibility == "private" {
        tags.push(vec!["private".into()]);
    } else {
        // Explicit "public" tag complements NIP-29's absence-of-"private"
        // convention, making channel visibility self-describing for clients.
        tags.push(vec!["public".into()]);
    }
    // NIP-29 hidden tag: hint to clients not to show DMs in public group lists.
    // Not a security boundary — access control is channel-scoped storage.
    if channel.channel_type == "dm" {
        tags.push(vec!["hidden".into()]);
        for m in members {
            tags.push(vec!["p".into(), hex::encode(&m.pubkey)]);
        }
    }
    // Buzz channels always require explicit membership.
    tags.push(vec!["closed".into()]);
    // Channel type so clients can distinguish stream/forum/dm without inference.
    tags.push(vec!["t".into(), channel.channel_type.clone()]);
    if let Some(topic) = channel.topic.as_ref().filter(|t| !t.is_empty()) {
        tags.push(vec!["topic".into(), topic.clone()]);
    }
    if let Some(purpose) = channel.purpose.as_ref().filter(|p| !p.is_empty()) {
        tags.push(vec!["purpose".into(), purpose.clone()]);
    }
    // Archived state — clients use this to hide channels from the sidebar.
    if channel.archived_at.is_some() {
        tags.push(vec!["archived".into(), "true".into()]);
    }
    // Ephemeral channel TTL — clients use this to show countdown timers.
    if let Some(ttl) = channel.ttl_seconds {
        tags.push(vec!["ttl".into(), ttl.to_string()]);
    }
    if let Some(deadline) = channel.ttl_deadline {
        tags.push(vec!["ttl_deadline".into(), deadline.to_rfc3339()]);
    }

    tags
}

/// kind:39001 (NIP-29 group admins) tags — owners and admins only.
pub fn nip29_admins_tags(channel_id: Uuid, members: &[MemberRecord]) -> Vec<Vec<String>> {
    let mut tags: Vec<Vec<String>> = vec![vec!["d".into(), channel_id.to_string()]];
    for m in members
        .iter()
        .filter(|m| m.role == "owner" || m.role == "admin")
    {
        tags.push(vec!["p".into(), hex::encode(&m.pubkey), m.role.clone()]);
    }
    tags
}

/// kind:39002 (NIP-29 group members) tags — every member, with role.
pub fn nip29_members_tags(channel_id: Uuid, members: &[MemberRecord]) -> Vec<Vec<String>> {
    let mut tags: Vec<Vec<String>> = vec![vec!["d".into(), channel_id.to_string()]];
    for m in members {
        // NIP-29 convention: ["p", pubkey, relay_url, role]. Empty relay_url
        // because the canonical relay is implicit (this event is signed by it).
        tags.push(vec![
            "p".into(),
            hex::encode(&m.pubkey),
            String::new(),
            m.role.clone(),
        ]);
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn channel() -> ChannelRecord {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        ChannelRecord {
            id: Uuid::nil(),
            name: "general".into(),
            channel_type: "stream".into(),
            visibility: "private".into(),
            description: None,
            canvas: None,
            created_by: vec![1],
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
            nip29_group_id: None,
            topic_required: false,
            max_members: None,
            topic: None,
            topic_set_by: None,
            topic_set_at: None,
            purpose: None,
            purpose_set_by: None,
            purpose_set_at: None,
            ttl_seconds: None,
            ttl_deadline: None,
        }
    }

    fn has(tags: &[Vec<String>], key: &str) -> bool {
        tags.iter().any(|t| t[0] == key)
    }

    fn value(tags: &[Vec<String>], key: &str) -> Option<String> {
        tags.iter()
            .find(|t| t[0] == key)
            .and_then(|t| t.get(1))
            .cloned()
    }

    #[test]
    fn live_channel_carries_no_archived_tag() {
        let tags = nip29_metadata_tags(&channel(), &[]);
        assert!(!has(&tags, "archived"));
        assert_eq!(
            value(&tags, "d").as_deref(),
            Some(Uuid::nil().to_string().as_str())
        );
        assert!(has(&tags, "private"));
        assert!(has(&tags, "closed"));
    }

    #[test]
    fn archived_channel_says_so() {
        let mut c = channel();
        c.archived_at = Some(Utc.with_ymd_and_hms(2026, 2, 2, 0, 0, 0).unwrap());
        let tags = nip29_metadata_tags(&c, &[]);
        assert_eq!(value(&tags, "archived").as_deref(), Some("true"));
    }

    #[test]
    fn ephemeral_channel_carries_its_ttl() {
        let mut c = channel();
        c.ttl_seconds = Some(3600);
        c.ttl_deadline = Some(Utc.with_ymd_and_hms(2026, 2, 2, 3, 4, 5).unwrap());
        let tags = nip29_metadata_tags(&c, &[]);
        assert_eq!(value(&tags, "ttl").as_deref(), Some("3600"));
        assert_eq!(
            value(&tags, "ttl_deadline").as_deref(),
            Some("2026-02-02T03:04:05+00:00")
        );
    }

    #[test]
    fn topic_and_purpose_survive_when_set() {
        let mut c = channel();
        c.topic = Some("ship it".into());
        c.purpose = Some("release coordination".into());
        c.description = Some("".into());
        let tags = nip29_metadata_tags(&c, &[]);
        assert_eq!(value(&tags, "topic").as_deref(), Some("ship it"));
        assert_eq!(
            value(&tags, "purpose").as_deref(),
            Some("release coordination")
        );
        assert!(!has(&tags, "about"), "an empty description is not a tag");
    }

    fn member(byte: u8, role: &str) -> MemberRecord {
        MemberRecord {
            channel_id: Uuid::nil(),
            pubkey: vec![byte],
            role: role.into(),
            joined_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            invited_by: None,
            removed_at: None,
        }
    }

    #[test]
    fn dm_inlines_its_participants() {
        let mut c = channel();
        c.channel_type = "dm".into();
        let tags = nip29_metadata_tags(&c, &[member(0xab, "member")]);
        assert!(has(&tags, "hidden"));
        assert_eq!(value(&tags, "p").as_deref(), Some("ab"));
    }

    #[test]
    fn a_non_dm_never_inlines_members() {
        let tags = nip29_metadata_tags(&channel(), &[member(0xab, "owner")]);
        assert!(!has(&tags, "hidden"));
        assert!(!has(&tags, "p"), "only DMs inline participants");
    }

    #[test]
    fn admins_tags_hold_only_owners_and_admins() {
        let members = [
            member(0x01, "owner"),
            member(0x02, "member"),
            member(0x03, "admin"),
        ];
        let tags = nip29_admins_tags(Uuid::nil(), &members);
        let listed: Vec<_> = tags.iter().filter(|t| t[0] == "p").collect();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0][1..], ["01".to_string(), "owner".to_string()]);
        assert_eq!(listed[1][1..], ["03".to_string(), "admin".to_string()]);
    }

    #[test]
    fn members_tags_hold_everyone_with_an_empty_relay_slot() {
        let tags = nip29_members_tags(
            Uuid::nil(),
            &[member(0x01, "owner"), member(0x02, "member")],
        );
        let listed: Vec<_> = tags.iter().filter(|t| t[0] == "p").collect();
        assert_eq!(listed.len(), 2);
        assert_eq!(
            listed[1][1..],
            ["02".to_string(), String::new(), "member".to_string()]
        );
    }
}
