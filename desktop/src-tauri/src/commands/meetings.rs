//! Meetings: transcripts + summaries for huddles.
//!
//! A huddle (voice/video session) is identified by its `KIND_HUDDLE_STARTED`
//! event id. During the huddle, whatever performs speech-to-text (agents bring
//! their own STT) emits `KIND_HUDDLE_TRANSCRIPT` (48107) segments tagged with
//! the huddle id (`e`) and channel (`h`). After it ends, a summary is posted as
//! `KIND_HUDDLE_SUMMARY` (48108). These commands record and read those events;
//! the audio→text and text→summary producers run on the live stack.

use nostr::{EventBuilder, Kind, Tag};
use serde::Serialize;
use tauri::State;

use crate::app_state::AppState;
use crate::relay::{query_relay, submit_event_with_keys};

const KIND_HUDDLE_STARTED: u16 = 48100;
const KIND_HUDDLE_TRANSCRIPT: u16 = 48107;
const KIND_HUDDLE_SUMMARY: u16 = 48108;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestHuddle {
    /// The huddle id — the `KIND_HUDDLE_STARTED` event id transcripts tag.
    pub huddle_id: String,
    pub started_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    /// Speaker pubkey (hex) — the event author.
    pub speaker: String,
    pub text: String,
    /// Unix seconds when the segment was recorded.
    pub at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSummary {
    pub text: String,
    pub at: i64,
    /// Author pubkey (hex) — the agent/user that produced the summary.
    pub author: String,
}

fn owner_keys(state: &AppState) -> Result<nostr::Keys, String> {
    Ok(state.keys.lock().map_err(|e| e.to_string())?.clone())
}

/// Find a channel's most recent huddle (to show its meeting notes).
#[tauri::command]
pub async fn get_latest_huddle(
    state: State<'_, AppState>,
    channel_id: String,
) -> Result<Option<LatestHuddle>, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [KIND_HUDDLE_STARTED],
            "#h": [channel_id],
            "limit": 20,
        })],
    )
    .await?;
    let latest = events
        .iter()
        .max_by_key(|event| event.created_at.as_secs())
        .map(|event| LatestHuddle {
            huddle_id: event.id.to_hex(),
            started_at: event.created_at.as_secs() as i64,
        });
    Ok(latest)
}

/// Read a huddle's transcript segments, ordered by time.
#[tauri::command]
pub async fn get_meeting_transcript(
    state: State<'_, AppState>,
    huddle_id: String,
) -> Result<Vec<TranscriptSegment>, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [KIND_HUDDLE_TRANSCRIPT],
            "#e": [huddle_id],
            "limit": 2000,
        })],
    )
    .await?;
    let mut segments: Vec<TranscriptSegment> = events
        .iter()
        .map(|event| TranscriptSegment {
            speaker: event.pubkey.to_hex(),
            text: event.content.clone(),
            at: event.created_at.as_secs() as i64,
        })
        .collect();
    segments.sort_by_key(|segment| segment.at);
    Ok(segments)
}

/// Read a huddle's latest meeting summary, if one has been posted.
#[tauri::command]
pub async fn get_meeting_summary(
    state: State<'_, AppState>,
    huddle_id: String,
) -> Result<Option<MeetingSummary>, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [KIND_HUDDLE_SUMMARY],
            "#e": [huddle_id],
            "limit": 10,
        })],
    )
    .await?;
    let latest = events
        .iter()
        .max_by_key(|event| event.created_at.as_secs())
        .map(|event| MeetingSummary {
            text: event.content.clone(),
            at: event.created_at.as_secs() as i64,
            author: event.pubkey.to_hex(),
        });
    Ok(latest)
}

/// Record one transcript segment for a huddle. STT producers call this per
/// spoken line; the segment is a channel-scoped event tagged with the huddle.
#[tauri::command]
pub async fn record_transcript_segment(
    state: State<'_, AppState>,
    channel_id: String,
    huddle_id: String,
    text: String,
) -> Result<(), String> {
    let keys = owner_keys(&state)?;
    let builder = EventBuilder::new(Kind::Custom(KIND_HUDDLE_TRANSCRIPT), text).tags([
        Tag::parse(["e", &huddle_id]).map_err(|e| e.to_string())?,
        Tag::parse(["h", &channel_id]).map_err(|e| e.to_string())?,
    ]);
    submit_event_with_keys(builder, &state, &keys, None).await?;
    Ok(())
}

/// Post a meeting summary for a huddle to its channel.
#[tauri::command]
pub async fn post_meeting_summary(
    state: State<'_, AppState>,
    channel_id: String,
    huddle_id: String,
    text: String,
) -> Result<(), String> {
    let keys = owner_keys(&state)?;
    let builder = EventBuilder::new(Kind::Custom(KIND_HUDDLE_SUMMARY), text).tags([
        Tag::parse(["e", &huddle_id]).map_err(|e| e.to_string())?,
        Tag::parse(["h", &channel_id]).map_err(|e| e.to_string())?,
    ]);
    submit_event_with_keys(builder, &state, &keys, None).await?;
    Ok(())
}
