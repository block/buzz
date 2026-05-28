use std::collections::HashMap;

use serde::{Deserialize, Deserializer, Serialize};

use sprout_core::PresenceStatus;

#[derive(Serialize)]
pub struct IdentityInfo {
    pub pubkey: String,
    pub display_name: String,
}

#[derive(Serialize, Deserialize)]
pub struct ProfileInfo {
    pub pubkey: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub about: Option<String>,
    pub nip05_handle: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct UserProfileSummaryInfo {
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub nip05_handle: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct UsersBatchResponse {
    pub profiles: HashMap<String, UserProfileSummaryInfo>,
    pub missing: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct UserSearchResultInfo {
    pub pubkey: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub nip05_handle: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct SearchUsersResponse {
    pub users: Vec<UserSearchResultInfo>,
}

#[derive(Serialize, Deserialize)]
pub struct UserNoteInfo {
    pub id: String,
    pub pubkey: String,
    pub created_at: i64,
    pub content: String,
    pub tags: Vec<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
pub struct NoteReactionSummary {
    pub note_id: String,
    pub emoji: String,
    pub count: usize,
    pub pubkeys: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct UserNotesCursor {
    pub before: i64,
    pub before_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct UserNotesResponse {
    pub notes: Vec<UserNoteInfo>,
    pub next_cursor: Option<UserNotesCursor>,
}

#[derive(Serialize, Deserialize)]
pub struct SetPresenceResponse {
    pub status: PresenceStatus,
    pub ttl_seconds: u64,
}

#[derive(Serialize, Deserialize)]
pub struct ChannelInfo {
    pub id: String,
    pub name: String,
    pub channel_type: String,
    pub visibility: String,
    #[serde(deserialize_with = "deserialize_null_string_as_empty")]
    pub description: String,
    pub topic: Option<String>,
    pub purpose: Option<String>,
    pub member_count: i64,
    #[serde(default)]
    pub member_pubkeys: Vec<String>,
    pub last_message_at: Option<String>,
    pub archived_at: Option<String>,
    #[serde(default)]
    pub participants: Vec<String>,
    #[serde(default)]
    pub participant_pubkeys: Vec<String>,
    #[serde(default = "default_true")]
    pub is_member: bool,
    pub ttl_seconds: Option<i32>,
    pub ttl_deadline: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct ChannelDetailInfo {
    pub id: String,
    pub name: String,
    pub channel_type: String,
    pub visibility: String,
    #[serde(deserialize_with = "deserialize_null_string_as_empty")]
    pub description: String,
    pub topic: Option<String>,
    pub topic_set_by: Option<String>,
    pub topic_set_at: Option<String>,
    pub purpose: Option<String>,
    pub purpose_set_by: Option<String>,
    pub purpose_set_at: Option<String>,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
    pub member_count: i64,
    pub topic_required: bool,
    pub max_members: Option<i32>,
    pub nip29_group_id: Option<String>,
    pub ttl_seconds: Option<i32>,
    pub ttl_deadline: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct ChannelMemberInfo {
    pub pubkey: String,
    pub role: String,
    /// Optional — kind:39002 events do not carry per-member join timestamps,
    /// so this is `None` when populated from a NIP-29 members event.
    #[serde(default)]
    pub joined_at: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct ChannelMembersResponse {
    pub members: Vec<ChannelMemberInfo>,
    pub next_cursor: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct FeedItemInfo {
    pub id: String,
    pub kind: u32,
    pub pubkey: String,
    pub content: String,
    pub created_at: u64,
    pub channel_id: Option<String>,
    pub channel_name: String,
    #[serde(default)]
    pub channel_type: Option<String>,
    pub tags: Vec<Vec<String>>,
    pub category: String,
}

#[derive(Serialize, Deserialize)]
pub struct FeedSections {
    pub mentions: Vec<FeedItemInfo>,
    pub needs_action: Vec<FeedItemInfo>,
    pub activity: Vec<FeedItemInfo>,
    pub agent_activity: Vec<FeedItemInfo>,
}

#[derive(Serialize, Deserialize)]
pub struct FeedMeta {
    pub since: i64,
    pub total: u64,
    pub generated_at: i64,
}

#[derive(Serialize, Deserialize)]
pub struct FeedResponse {
    pub feed: FeedSections,
    pub meta: FeedMeta,
}

#[derive(Serialize, Deserialize)]
pub struct SearchHitInfo {
    pub event_id: String,
    pub content: String,
    pub kind: u32,
    pub pubkey: String,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub created_at: u64,
    pub score: f64,
}

#[derive(Serialize, Deserialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHitInfo>,
    pub found: u64,
}

#[derive(Serialize, Deserialize)]
pub struct SendChannelMessageResponse {
    pub event_id: String,
    pub parent_event_id: Option<String>,
    pub root_event_id: Option<String>,
    pub depth: u32,
    pub created_at: i64,
}

#[derive(Serialize, Deserialize)]
pub struct ThreadSummary {
    pub reply_count: u32,
    pub descendant_count: u32,
    pub last_reply_at: Option<i64>,
    pub participants: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct ForumMessageInfo {
    pub event_id: String,
    pub pubkey: String,
    pub content: String,
    pub kind: u32,
    pub created_at: i64,
    pub channel_id: String,
    pub tags: Vec<Vec<String>>,
    #[serde(default)]
    pub thread_summary: Option<ThreadSummary>,
    #[serde(default)]
    pub reactions: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
pub struct ForumPostsResponse {
    pub messages: Vec<ForumMessageInfo>,
    pub next_cursor: Option<i64>,
}

#[derive(Serialize, Deserialize)]
pub struct ForumThreadReplyInfo {
    pub event_id: String,
    pub pubkey: String,
    pub content: String,
    pub kind: u32,
    pub created_at: i64,
    pub channel_id: String,
    pub tags: Vec<Vec<String>>,
    pub parent_event_id: Option<String>,
    pub root_event_id: Option<String>,
    pub depth: u32,
    pub broadcast: bool,
    #[serde(default)]
    pub reactions: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
pub struct ForumThreadResponse {
    pub root: ForumMessageInfo,
    pub replies: Vec<ForumThreadReplyInfo>,
    pub total_replies: u32,
    pub next_cursor: Option<String>,
}

fn deserialize_null_string_as_empty<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

fn default_true() -> bool {
    true
}

// ── Social / Contact list ───────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
pub struct ContactListResponse {
    pub id: String,
    pub pubkey: String,
    pub created_at: i64,
    pub tags: Vec<Vec<String>>,
    pub content: String,
}

#[derive(Serialize, Deserialize)]
pub struct ContactEntry {
    pub pubkey: String,
    #[serde(default)]
    pub relay_url: Option<String>,
    #[serde(default)]
    pub petname: Option<String>,
}
